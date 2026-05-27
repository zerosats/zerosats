// Deploy the rollup proxy + initialize + transfer ProxyAdmin ownership to
// the in-contract timelock. Mainnet-shaped: no devnet key fallback, every
// non-default input is explicit, every post-deploy invariant is verified
// against the chain before exit.

import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import aliceTokenArtifact from "../artifacts/contracts/helper/AliceERC20.sol/AliceERC20.json";
import proxyArtifact from "../openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol/TransparentUpgradeableProxy.json";

import { readFile } from "fs/promises";
import {
  createPublicClient,
  createWalletClient,
  http,
  formatEther,
  encodeFunctionData,
  getContract,
  maxUint256,
  parseAbi,
} from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  parseNetwork,
  requireEnv,
  resolveRpcUrl,
  wcbtcAddressFor,
} from "./shared";
import IERC20Artifact from "../openzeppelin-contracts/token/ERC20/IERC20.json";

const EIP1967_ADMIN_STORAGE_SLOT =
  "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";

const PROXY_ADMIN_ABI = parseAbi([
  "function owner() view returns (address)",
  "function transferOwnership(address newOwner)",
]);

const IERC20_METADATA_ABI = parseAbi([
  "function totalSupply() view returns (uint256)",
  "function decimals() view returns (uint8)",
]);

const ONE_HOUR_SECONDS = 3_600n;
const SEVEN_DAYS_SECONDS = 7n * 24n * ONE_HOUR_SECONDS;
const DEFAULT_PER_MINT_CAP_WEI = 1_000_000_000_000_000n; // 0.001 token
const DEFAULT_GLOBAL_TVL_CAP_WEI = 10_000_000_000_000_000_000n; // 10 token
const SATS_TO_WEI = 10_000_000_000n; // 18-dec BTC wrappers
const DEFAULT_BURN_FEE_WEI = 300n * SATS_TO_WEI;
const MAX_BURN_FEE_WEI = 3_000n * SATS_TO_WEI;

const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";

// Encoded noteKind seed for the initial token. Canonical layout (must match
// the noir circuits — see pkg/zk-primitives::note_kind):
//   bytes 0..1   : 0x0002 (kind discriminator)
//   bytes 2..7   : zero pad
//   bytes 8..9   : chainId (big-endian uint16)
//   bytes 10..29 : token address (20 bytes)
//   bytes 30..31 : 0x0000 (trailing pad)
// The contract's `initialize` enforces the address slice at bytes 10..29.
// Operators can override entirely via INITIAL_NOTE_KIND env if the noir
// circuits are regenerated with a different layout.
function deriveInitialNoteKind(
  chainId: number,
  tokenAddress: `0x${string}`,
): `0x${string}` {
  const prefix = "0002000000000000" + chainId.toString(16).padStart(4, "0");
  const tokenHex = tokenAddress.slice(2).toLowerCase().padStart(40, "0");
  const tail = "0000";
  return `0x${prefix}${tokenHex}${tail}`.toLowerCase() as `0x${string}`;
}

// Extract bytes 10..29 from a 32-byte 0x-prefixed noteKind, returned as a
// 0x-prefixed 20-byte lowercase hex string. Matches the address slice the
// contract validates against.
function noteKindAddressSlice(noteKind: `0x${string}`): `0x${string}` {
  const stripped = noteKind.slice(2).toLowerCase();
  if (stripped.length !== 64) {
    throw new Error(
      `INITIAL_NOTE_KIND must be 32 bytes (got ${stripped.length / 2})`,
    );
  }
  // Hex chars 20..59 inclusive = bytes 10..29 = 40 hex chars.
  return ("0x" + stripped.slice(20, 60)) as `0x${string}`;
}

function parseBigIntEnv(name: string, fallback: bigint): bigint {
  const value = process.env[name];
  if (!value || value.trim() === "") return fallback;
  try {
    return BigInt(value);
  } catch {
    throw new Error(`${name} must be an integer string, got: ${value}`);
  }
}

function parseAddressListEnv(
  name: string,
  fallback: `0x${string}`[],
): `0x${string}`[] {
  const value = process.env[name];
  if (!value || value.trim() === "") return fallback;
  const parsed = value
    .split(",")
    .map((v) => v.trim())
    .filter((v) => v.length > 0) as `0x${string}`[];
  if (parsed.length === 0) {
    throw new Error(`${name} must contain at least one address`);
  }
  return parsed;
}

function readAddressFromSlot(
  slotValue: `0x${string}` | undefined,
): `0x${string}` {
  if (!slotValue || slotValue.length < 66) {
    throw new Error(`Unexpected slot value: ${slotValue}`);
  }
  return `0x${slotValue.slice(26)}` as `0x${string}`;
}

async function readVkHash(): Promise<`0x${string}`> {
  const override = process.env.AGG_AGG_VERIFICATION_KEY_HASH;
  if (override) {
    if (!override.startsWith("0x") || override.length !== 66) {
      throw new Error(
        `AGG_AGG_VERIFICATION_KEY_HASH must be 32-byte 0x-prefixed hex`,
      );
    }
    return override as `0x${string}`;
  }
  const path = "contracts/noir/agg_agg_vk_hash.json";
  const raw = await readFile(path).catch((e) => {
    throw new Error(
      `Cannot read ${path}: ${e}. Set AGG_AGG_VERIFICATION_KEY_HASH env to override.`,
    );
  });
  const parsed = JSON.parse(raw.toString());
  const hash = parsed.vkHash ?? parsed.hash ?? parsed.aggAggVkHash;
  if (!hash || typeof hash !== "string" || !hash.startsWith("0x")) {
    throw new Error(
      `${path} did not contain a 'vkHash' field with a 0x-prefixed hash`,
    );
  }
  return hash as `0x${string}`;
}

async function main() {
  console.log("Initialization...");

  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);

  // Mainnet requires explicit confirmation of the chainId we are about to
  // sign against — protects against ENV mis-config or RPC switcheroo.
  if (profile.name === "main") {
    const confirm = process.env.CONFIRM;
    if (confirm !== String(profile.chainId)) {
      throw new Error(
        `CONFIRM must equal chainId ${profile.chainId} to run deploy.ts on mainnet`,
      );
    }
  }

  const aggregateVerifierAddr = requireEnv("VERIFIER") as `0x${string}`;
  const account =
    profile.name === "dev"
      ? privateKeyToAccount(requireEnv("PRIVATE_KEY") as `0x${string}`)
      : mnemonicToAccount(requireEnv("MNEMONIC"));

  let proverAddress: `0x${string}` =
    (process.env.PROVER_ADDRESS as `0x${string}`) || account.address;
  let validators: `0x${string}`[] = parseAddressListEnv("VALIDATORS", [
    account.address,
  ]);

  console.log(`    Network         - ${profile.name}`);
  console.log(`    Chain ID        - ${profile.chainId}`);
  console.log(`    Prover address  - ${proverAddress}`);
  console.log(`    Validators      - ${validators.join(",")}`);
  console.log(`    Verifier        - ${aggregateVerifierAddr}`);

  let ownerAddress = account.address;
  console.log(`    Owner           - ${ownerAddress}`);

  let claimerAddress = process.env.CLAIMER_ADDRESS as `0x${string}` | undefined;
  if (profile.name === "main" || profile.name === "test") {
    if (!claimerAddress) throw new Error("CLAIMER_ADDRESS is required");
  } else if (!claimerAddress) {
    claimerAddress = ownerAddress;
  }
  if (claimerAddress === ZERO_ADDRESS) {
    throw new Error("CLAIMER_ADDRESS cannot be zero address");
  }
  console.log(`    Claimer          - ${claimerAddress}`);

  // Init config: secure defaults overridable via env vars.
  const perMintCap = parseBigIntEnv(
    "PER_MINT_CAP_WEI",
    DEFAULT_PER_MINT_CAP_WEI,
  );
  const globalTvlCap = parseBigIntEnv(
    "GLOBAL_TVL_CAP_WEI",
    DEFAULT_GLOBAL_TVL_CAP_WEI,
  );
  const openProvingDelay = parseBigIntEnv(
    "OPEN_PROVING_DELAY_SECONDS",
    SEVEN_DAYS_SECONDS,
  );
  const burnFee = parseBigIntEnv("BURN_FEE_WEI", DEFAULT_BURN_FEE_WEI);
  const feeSink =
    (process.env.FEE_SINK as `0x${string}` | undefined) ?? ownerAddress;
  const timelockMinDelay = parseBigIntEnv(
    "TIMELOCK_MIN_DELAY_SECONDS",
    ONE_HOUR_SECONDS,
  );
  const timelockProposers = parseAddressListEnv("TIMELOCK_PROPOSERS", [
    ownerAddress,
  ]);
  const timelockExecutors = parseAddressListEnv("TIMELOCK_EXECUTORS", [
    ownerAddress,
  ]);

  if (feeSink === ZERO_ADDRESS) {
    throw new Error("FEE_SINK cannot be zero address");
  }
  if (burnFee > MAX_BURN_FEE_WEI) {
    throw new Error(
      `BURN_FEE_WEI exceeds max (${MAX_BURN_FEE_WEI.toString()} wei = 3000 sats)`,
    );
  }

  console.log(`🚀 Connecting to Citrea (${rpcUrl})...`);

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

  await assertChainId(publicClient, profile.chainId, "deploy");

  const walletClient = createWalletClient({
    account,
    chain: {
      ...profile.chain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 60_000, retryCount: 3 }),
  });

  const blockNumber = await publicClient.getBlockNumber();
  console.log(`✅ Block Number: ${blockNumber}`);
  let balance = await publicClient.getBalance({ address: account.address });
  console.log(`✅ Account: ${account.address}`);
  console.log(`✅ Balance: ${formatEther(balance)} cBTC`);

  // ── Preflight ──
  const verifierCode = await publicClient.getCode({
    address: aggregateVerifierAddr,
  });
  if (!verifierCode || verifierCode === "0x") {
    throw new Error(
      `No bytecode at VERIFIER=${aggregateVerifierAddr} — deploy verifier first`,
    );
  }

  const vkHash = await readVkHash();
  console.log(`✅ Verifier code:    ${(verifierCode.length - 2) / 2} bytes`);
  console.log(`✅ Aggregate VK hash: ${vkHash}`);

  // Resolve the ERC20 token address.
  let erc20Address: `0x${string}`;
  if (profile.name === "dev") {
    console.log("\n🔍 Devnet: deploying fixture ERC20...");
    const erc20Tx = await walletClient.deployContract({
      abi: aliceTokenArtifact.abi,
      bytecode: aliceTokenArtifact.bytecode,
      args: [maxUint256],
    });
    const erc20Receipt = await publicClient.waitForTransactionReceipt({
      hash: erc20Tx,
    });
    if (erc20Receipt.status !== "success") {
      throw new Error("ERC20 deploy reverted");
    }
    erc20Address = erc20Receipt.contractAddress!;
  } else {
    const fromEnv = process.env.ERC20_ADDRESS as `0x${string}` | undefined;
    erc20Address = fromEnv ?? wcbtcAddressFor(profile);
  }
  console.log(`✅ ERC20 token: ${erc20Address}`);

  const erc20Code = await publicClient.getCode({ address: erc20Address });
  if (!erc20Code || erc20Code === "0x") {
    throw new Error(`No bytecode at ERC20_ADDRESS=${erc20Address}`);
  }
  const tokenMeta = getContract({
    address: erc20Address,
    abi: IERC20_METADATA_ABI,
    client: { public: publicClient },
  });
  const decimals = (await tokenMeta.read.decimals()) as number;
  if (decimals !== 18) {
    throw new Error(
      `ERC20 at ${erc20Address} reports decimals=${decimals}; expected 18`,
    );
  }
  await tokenMeta.read.totalSupply(); // sanity-call, throws if missing.

  // Initial noteKind: env override wins; otherwise derive from chain + token.
  const initialNoteKind =
    (process.env.INITIAL_NOTE_KIND as `0x${string}` | undefined) ??
    deriveInitialNoteKind(profile.chainId, erc20Address);
  const noteKindAddrSlice = noteKindAddressSlice(initialNoteKind);
  if (noteKindAddrSlice !== erc20Address.toLowerCase()) {
    throw new Error(
      `INITIAL_NOTE_KIND bytes 10..29 (${noteKindAddrSlice}) do not match ERC20 address (${erc20Address.toLowerCase()})`,
    );
  }
  console.log(`✅ Initial noteKind: ${initialNoteKind}`);

  console.log(
    `\n🔍 Init params: perMintCap=${perMintCap} globalTvlCap=${globalTvlCap}`,
  );
  console.log(
    `   openProvingDelay=${openProvingDelay}s burnFee=${burnFee} wei feeSink=${feeSink}`,
  );
  console.log(
    `   timelockDelay=${timelockMinDelay}s proposers=${timelockProposers.join(",")} executors=${timelockExecutors.join(",")}`,
  );

  // ── Deploy implementation ──
  console.log("\n🔍 Deploying Rollup implementation...");
  const rollupV1Tx = await walletClient.deployContract({
    abi: rollupV1Artifact.abi,
    bytecode: rollupV1Artifact.bytecode,
  });
  let receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupV1Tx,
  });
  if (receipt.status !== "success") {
    throw new Error("RollupV1 implementation deploy reverted");
  }
  const rollupAddress = receipt.contractAddress!;
  console.log(`✅ Rollup Contract (Implementation): ${rollupAddress}`);

  // ── Deploy proxy + initialize ──
  const rollupInitializeCalldata = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName: "initialize",
    args: [
      ownerAddress,
      claimerAddress,
      erc20Address,
      initialNoteKind,
      aggregateVerifierAddr,
      proverAddress,
      validators,
      vkHash,
      perMintCap,
      globalTvlCap,
      openProvingDelay,
      burnFee,
      feeSink,
      timelockMinDelay,
      timelockProposers,
      timelockExecutors,
    ],
  });

  const rollupProxyTx = await walletClient.deployContract({
    abi: proxyArtifact.abi,
    bytecode: proxyArtifact.bytecode,
    args: [rollupAddress, ownerAddress, rollupInitializeCalldata],
  });
  receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupProxyTx,
  });
  if (receipt.status !== "success") {
    throw new Error("RollupV1 proxy deploy reverted");
  }
  const rollupProxyAddr = receipt.contractAddress!;
  console.log(`✅ Rollup Contract (Proxy): ${rollupProxyAddr}`);

  // ── ProxyAdmin discovery + handoff ──
  const adminSlot = await publicClient.getStorageAt({
    address: rollupProxyAddr,
    slot: EIP1967_ADMIN_STORAGE_SLOT,
  });
  const proxyAdminAddress = readAddressFromSlot(adminSlot);
  console.log(`✅ Rollup Proxy Admin: ${proxyAdminAddress}`);

  const rollup = getContract({
    address: rollupProxyAddr,
    abi: rollupV1Artifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });

  const aliceToken = getContract({
    address: erc20Address,
    abi: IERC20Artifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });

  // Approve the proxy on the operator's behalf so the operator can mint
  // immediately. (Mainnet operators that don't intend to mint can skip
  // this by setting SKIP_OPERATOR_APPROVAL=1.)
  if (process.env.SKIP_OPERATOR_APPROVAL !== "1") {
    console.log("\n🔍 Approving ERC20 spending for proxy...");
    const approveHash = await aliceToken.write.approve(
      [rollupProxyAddr, maxUint256],
      { gas: 1_000_000n },
    );
    const approveReceipt = await publicClient.waitForTransactionReceipt({
      hash: approveHash,
    });
    if (approveReceipt.status !== "success") {
      throw new Error("ERC20 approve reverted");
    }
    console.log(`✅ Approved maxUint256: ${approveHash}`);
  }

  // ── Verify ownership invariants ──
  const timelockAddress = (await rollup.read.timelock()) as `0x${string}`;
  const rollupOwner = (await rollup.read.owner()) as `0x${string}`;
  if (rollupOwner.toLowerCase() !== timelockAddress.toLowerCase()) {
    throw new Error(
      `Rollup owner mismatch after init. owner=${rollupOwner} timelock=${timelockAddress}`,
    );
  }
  console.log(`✅ Rollup owner is timelock: ${timelockAddress}`);

  const proxyAdmin = getContract({
    address: proxyAdminAddress,
    abi: PROXY_ADMIN_ABI,
    client: { public: publicClient, wallet: walletClient },
  });

  const proxyAdminOwnerBefore =
    (await proxyAdmin.read.owner()) as `0x${string}`;
  console.log(`✅ ProxyAdmin owner before handoff: ${proxyAdminOwnerBefore}`);

  if (proxyAdminOwnerBefore.toLowerCase() !== timelockAddress.toLowerCase()) {
    if (proxyAdminOwnerBefore.toLowerCase() !== ownerAddress.toLowerCase()) {
      throw new Error(
        `Unexpected ProxyAdmin owner ${proxyAdminOwnerBefore}; expected ${ownerAddress} before transfer`,
      );
    }
    const transferHash = await proxyAdmin.write.transferOwnership(
      [timelockAddress],
      { account },
    );
    const transferReceipt = await publicClient.waitForTransactionReceipt({
      hash: transferHash,
    });
    if (transferReceipt.status !== "success") {
      throw new Error("ProxyAdmin transferOwnership reverted");
    }
    console.log(
      `✅ ProxyAdmin ownership transferred to timelock: ${transferHash}`,
    );
  } else {
    console.log("✅ ProxyAdmin ownership already timelocked");
  }

  const proxyAdminOwnerAfter = (await proxyAdmin.read.owner()) as `0x${string}`;
  if (proxyAdminOwnerAfter.toLowerCase() !== timelockAddress.toLowerCase()) {
    throw new Error(
      `ProxyAdmin owner mismatch after transfer. owner=${proxyAdminOwnerAfter} timelock=${timelockAddress}`,
    );
  }
  console.log(`✅ ProxyAdmin owner now timelock: ${proxyAdminOwnerAfter}`);

  // ── Post-deploy readback. Every value below comes from the chain, not
  // the inputs — mismatches throw before we print DEPLOY_OUTPUT. ──
  const onChainToken = (await rollup.read.token()) as `0x${string}`;
  // zkVerifierKeys is a public bytes32[] array; the seed VK hash is index 0.
  const onChainVerifierKeyHash = (await rollup.read.zkVerifierKeys([
    0n,
  ])) as `0x${string}`;
  const onChainPerMint = (await rollup.read.perMintCap()) as bigint;
  const onChainTvl = (await rollup.read.globalTvlCap()) as bigint;
  const onChainOpenDelay = (await rollup.read.openProvingDelay()) as bigint;
  const onChainBurnFee = (await rollup.read.burnFee()) as bigint;
  const onChainFeeSink = (await rollup.read.feeSink()) as `0x${string}`;
  const onChainSubstitutor = (await rollup.read.burnSubstitutors([
    ownerAddress,
  ])) as boolean;

  const mismatches: string[] = [];
  if (onChainToken.toLowerCase() !== erc20Address.toLowerCase())
    mismatches.push(`token (got=${onChainToken} want=${erc20Address})`);
  if (onChainVerifierKeyHash.toLowerCase() !== vkHash.toLowerCase())
    mismatches.push(
      `vkHash (got=${onChainVerifierKeyHash} want=${vkHash})`,
    );
  if (onChainPerMint !== perMintCap)
    mismatches.push(`perMintCap (got=${onChainPerMint} want=${perMintCap})`);
  if (onChainTvl !== globalTvlCap)
    mismatches.push(`globalTvlCap (got=${onChainTvl} want=${globalTvlCap})`);
  if (onChainOpenDelay !== openProvingDelay)
    mismatches.push(
      `openProvingDelay (got=${onChainOpenDelay} want=${openProvingDelay})`,
    );
  if (onChainBurnFee !== burnFee)
    mismatches.push(`burnFee (got=${onChainBurnFee} want=${burnFee})`);
  if (onChainFeeSink.toLowerCase() !== feeSink.toLowerCase())
    mismatches.push(`feeSink (got=${onChainFeeSink} want=${feeSink})`);
  if (!onChainSubstitutor)
    mismatches.push(`burnSubstitutors[owner] should be true after init`);

  if (mismatches.length > 0) {
    throw new Error(
      `Post-deploy readback failed:\n - ${mismatches.join("\n - ")}`,
    );
  }
  console.log("✅ Post-deploy readback OK");

  // Machine-readable output for downstream automation.
  console.log(
    `DEPLOY_OUTPUT=${JSON.stringify({
      network: profile.name,
      chainId: profile.chainId,
      rollupProxy: rollupProxyAddr,
      rollupImplementation: rollupAddress,
      rollupOwner,
      timelock: timelockAddress,
      proxyAdmin: proxyAdminAddress,
      proxyAdminOwner: proxyAdminOwnerAfter,
      initialNoteKind,
      perMintCap: perMintCap.toString(),
      globalTvlCap: globalTvlCap.toString(),
      openProvingDelaySeconds: openProvingDelay.toString(),
      burnFeeWei: burnFee.toString(),
      feeSink,
      burner: claimerAddress,
      erc20: erc20Address,
      verifier: aggregateVerifierAddr,
      vkHash,
    })}`,
  );
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
