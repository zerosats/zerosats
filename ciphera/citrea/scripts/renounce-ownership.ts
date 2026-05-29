/**
 * Renounce ownership of the Rollup proxy and its ProxyAdmin (sets both to
 * the zero address). After this, no upgrades or owner-gated settings can
 * ever be applied. Mainnet kill-switch — do not run on day one.
 *
 * Behaviour:
 *  - If the rollup is already owned by 0x0, only the ProxyAdmin is
 *    renounced. If the ProxyAdmin is already 0x0, only the rollup.
 *  - If a contract is owned by an EOA matching the script's caller, the
 *    renunciation is sent directly.
 *  - If a contract is owned by a contract (e.g. the TimelockController
 *    deployed by RollupV1.initialize), the renunciation is dispatched
 *    via the timelock using the same schedule/execute helper as the
 *    rest of the deploy scripts.
 *
 * Required env:
 *   NETWORK              dev|test|main
 *   ROLLUP_PROXY_ADDRESS
 *   MODE                 schedule|execute|auto
 *   PRIVATE_KEY          (dev only) or MNEMONIC (test/main)
 *
 * Mainnet safety:
 *   CONFIRM_RENOUNCE     must equal the chainId (4114 for mainnet)
 *   DRY_RUN=1            prints calldata + opIds, does not send any tx
 */

import {
  createPublicClient,
  createWalletClient,
  encodeFunctionData,
  encodeAbiParameters,
  getContract,
  http,
  keccak256,
  parseAbi,
  zeroAddress,
  zeroHash,
} from "viem";
import type { PublicClient, WalletClient } from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  parseNetwork,
  parseTimelockMode,
  requireEnv,
  resolveRpcUrl,
  TIMELOCK_ABI,
  timelockDispatch,
} from "./shared";

const EIP1967_ADMIN_STORAGE_SLOT =
  "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";

const OWNABLE_ABI = parseAbi([
  "function owner() view returns (address)",
  "function renounceOwnership()",
]);

function readAddressFromSlot(
  slotValue: `0x${string}` | undefined,
): `0x${string}` {
  if (!slotValue || slotValue.length < 66) {
    throw new Error(`Unexpected EIP-1967 admin slot value: ${slotValue}`);
  }
  return `0x${slotValue.slice(26)}` as `0x${string}`;
}

const isZero = (a: string) => a.toLowerCase() === zeroAddress.toLowerCase();

async function printReadback(params: {
  label: string;
  publicClient: PublicClient;
  rollupAddress: `0x${string}`;
  proxyAdminAddress: `0x${string}`;
  caller: `0x${string}`;
}): Promise<void> {
  const { label, publicClient, rollupAddress, proxyAdminAddress, caller } =
    params;
  const rollup = getContract({
    address: rollupAddress,
    abi: OWNABLE_ABI,
    client: { public: publicClient },
  });
  const proxyAdmin = getContract({
    address: proxyAdminAddress,
    abi: OWNABLE_ABI,
    client: { public: publicClient },
  });
  const chainId = await publicClient.getChainId();
  const block = await publicClient.getBlockNumber();
  const rollupOwner = (await rollup.read.owner()) as `0x${string}`;
  const proxyAdminOwner = (await proxyAdmin.read.owner()) as `0x${string}`;
  console.log(`\n── ${label} ──`);
  console.log(`  chainId:          ${chainId}`);
  console.log(`  block:            ${block}`);
  console.log(`  caller:           ${caller}`);
  console.log(`  rollup:           ${rollupAddress}`);
  console.log(`  rollupOwner:      ${rollupOwner}`);
  console.log(`  proxyAdmin:       ${proxyAdminAddress}`);
  console.log(`  proxyAdminOwner:  ${proxyAdminOwner}`);
}

async function maybeDryRunPrint(params: {
  label: string;
  publicClient: PublicClient;
  contractAddress: `0x${string}`;
  owner: `0x${string}`;
}): Promise<void> {
  const { label, publicClient, contractAddress, owner } = params;
  const data = encodeFunctionData({
    abi: OWNABLE_ABI,
    functionName: "renounceOwnership",
  });
  console.log(`\nDRY RUN — ${label}`);
  console.log(`  target:  ${contractAddress}`);
  console.log(`  data:    ${data}`);
  // If owned by a contract, compute the timelock opId so an operator can
  // confirm what would have been scheduled.
  const code = await publicClient.getCode({ address: owner });
  if (code && code !== "0x") {
    const tl = getContract({
      address: owner,
      abi: TIMELOCK_ABI,
      client: { public: publicClient },
    });
    const salt = keccak256(
      encodeAbiParameters(
        [{ type: "address" }, { type: "bytes" }],
        [contractAddress, data],
      ),
    );
    const opId = (await tl.read.hashOperation([
      contractAddress,
      0n,
      data,
      zeroHash,
      salt,
    ])) as `0x${string}`;
    console.log(`  via timelock: ${owner}`);
    console.log(`  salt:    ${salt}`);
    console.log(`  opId:    ${opId}`);
  } else {
    console.log(`  via EOA: ${owner}`);
  }
}

async function renounce(params: {
  label: string;
  contractAddress: `0x${string}`;
  currentOwner: `0x${string}`;
  caller: `0x${string}`;
  publicClient: PublicClient;
  walletClient: WalletClient;
  mode: "schedule" | "execute" | "auto";
}): Promise<void> {
  const {
    label,
    contractAddress,
    currentOwner,
    caller,
    publicClient,
    walletClient,
    mode,
  } = params;

  if (isZero(currentOwner)) {
    console.log(`⏭  ${label}: already renounced (owner=0x0)`);
    return;
  }

  const data = encodeFunctionData({
    abi: OWNABLE_ABI,
    functionName: "renounceOwnership",
  });

  if (currentOwner.toLowerCase() === caller.toLowerCase()) {
    console.log(
      `🚀 ${label}: caller is owner — sending direct renounceOwnership()`,
    );
    const hash = await walletClient.sendTransaction({
      to: contractAddress,
      data,
      gas: 200_000n,
    } as any);
    console.log(`📝 ${label}: tx ${hash}`);
    const receipt = await publicClient.waitForTransactionReceipt({ hash });
    if (receipt.status !== "success") {
      throw new Error(`${label}: renounceOwnership() reverted`);
    }
    console.log(`✅ ${label}: renounced in block ${receipt.blockNumber}`);
    return;
  }

  console.log(
    `🕒 ${label}: owner is a contract (${currentOwner}) — dispatching via timelock`,
  );
  await timelockDispatch({
    publicClient,
    walletClient,
    timelock: currentOwner,
    target: contractAddress,
    data,
    mode,
  });
}

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);
  const rollupProxyAddr = requireEnv("ROLLUP_PROXY_ADDRESS") as `0x${string}`;
  const mode = parseTimelockMode(process.env.MODE);
  const dryRun = process.env.DRY_RUN === "1";

  // Mainnet kill-switch gating.
  if (profile.name === "main" && !dryRun) {
    const confirm = process.env.CONFIRM_RENOUNCE;
    if (confirm !== String(profile.chainId)) {
      throw new Error(
        `CONFIRM_RENOUNCE must equal chainId ${profile.chainId} to run on mainnet`,
      );
    }
  }

  const account =
    profile.name === "dev"
      ? privateKeyToAccount(requireEnv("PRIVATE_KEY") as `0x${string}`)
      : mnemonicToAccount(requireEnv("MNEMONIC"));

  const publicClient = createPublicClient({
    chain: {
      ...profile.chain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 30_000, retryCount: 3 }),
  });

  await assertChainId(publicClient, profile.chainId, "renounce-ownership");

  const walletClient = createWalletClient({
    account,
    chain: {
      ...profile.chain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 30_000, retryCount: 3 }),
  });

  const adminSlot = await publicClient.getStorageAt({
    address: rollupProxyAddr,
    slot: EIP1967_ADMIN_STORAGE_SLOT,
  });
  const proxyAdminAddress = readAddressFromSlot(adminSlot);

  const rollup = getContract({
    address: rollupProxyAddr,
    abi: OWNABLE_ABI,
    client: { public: publicClient, wallet: walletClient },
  });
  const proxyAdmin = getContract({
    address: proxyAdminAddress,
    abi: OWNABLE_ABI,
    client: { public: publicClient, wallet: walletClient },
  });

  await printReadback({
    label: "Initial state",
    publicClient,
    rollupAddress: rollupProxyAddr,
    proxyAdminAddress,
    caller: account.address,
  });
  console.log(`  network:          ${profile.name}`);
  console.log(`  mode:             ${mode}`);
  console.log(`  dryRun:           ${dryRun}`);

  const rollupOwner = (await rollup.read.owner()) as `0x${string}`;
  const proxyAdminOwner = (await proxyAdmin.read.owner()) as `0x${string}`;

  if (isZero(rollupOwner) && isZero(proxyAdminOwner)) {
    console.log("\n✅ Both ownerships already renounced — nothing to do");
    return;
  }

  if (dryRun) {
    if (!isZero(rollupOwner)) {
      await maybeDryRunPrint({
        label: "Rollup",
        publicClient,
        contractAddress: rollupProxyAddr,
        owner: rollupOwner,
      });
    }
    if (!isZero(proxyAdminOwner)) {
      await maybeDryRunPrint({
        label: "ProxyAdmin",
        publicClient,
        contractAddress: proxyAdminAddress,
        owner: proxyAdminOwner,
      });
    }
    console.log("\nDRY RUN complete — no transactions sent");
    return;
  }

  await renounce({
    label: "Rollup",
    contractAddress: rollupProxyAddr,
    currentOwner: rollupOwner,
    caller: account.address,
    publicClient,
    walletClient,
    mode,
  });

  await renounce({
    label: "ProxyAdmin",
    contractAddress: proxyAdminAddress,
    currentOwner: proxyAdminOwner,
    caller: account.address,
    publicClient,
    walletClient,
    mode,
  });

  if (mode !== "schedule") {
    await printReadback({
      label: "Final state",
      publicClient,
      rollupAddress: rollupProxyAddr,
      proxyAdminAddress,
      caller: account.address,
    });
    const finalRollupOwner = (await rollup.read.owner()) as `0x${string}`;
    const finalProxyAdminOwner =
      (await proxyAdmin.read.owner()) as `0x${string}`;
    if (!isZero(finalRollupOwner) || !isZero(finalProxyAdminOwner)) {
      throw new Error(
        "Renunciation incomplete after execute — check timelock state",
      );
    }
    console.log("\n✅ Both ownerships are now address(0)");
  } else {
    console.log(
      "\nℹ️  Mode=schedule: ops are queued in the timelock. Re-run with MODE=execute after the min delay.",
    );
  }
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
