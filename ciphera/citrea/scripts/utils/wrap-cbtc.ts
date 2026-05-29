// Wrap native cBTC → WCBTC ERC20, then optionally transfer to a recipient.
// Operator convenience tool, NOT part of any production runbook.

import { createPublicClient, createWalletClient, formatEther, http, parseEther } from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  parseNetwork,
  requireEnv,
  resolveRpcUrl,
  wcbtcAddressFor,
  WCBTC_ADDRESS,
} from "../shared";

const WCBTC_ABI = [
  {
    name: "deposit",
    type: "function",
    stateMutability: "payable",
    inputs: [],
    outputs: [],
  },
  {
    name: "balanceOf",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "owner", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "transfer",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [
      { name: "to", type: "address" },
      { name: "value", type: "uint256" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
] as const;

const MAINNET_AMOUNT_WARN_CBTC = parseEther("0.01");

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);
  const amountToWrap = parseEther(requireEnv("AMOUNT_CBTC"));
  const transferTo = process.env.RECIPIENT as `0x${string}` | undefined;
  const transferAmountStr = process.env.RECIPIENT_AMOUNT_CBTC;

  if (transferTo && !transferAmountStr) {
    throw new Error(
      "RECIPIENT set but RECIPIENT_AMOUNT_CBTC missing — refusing to guess",
    );
  }
  const transferAmount = transferAmountStr
    ? parseEther(transferAmountStr)
    : undefined;

  // Token address: prefer env override, else the profile's canonical WCBTC,
  // else the legacy hardcoded testnet address (back-compat only).
  let wcbtcAddress: `0x${string}`;
  if (process.env.WCBTC_ADDRESS) {
    wcbtcAddress = process.env.WCBTC_ADDRESS as `0x${string}`;
  } else {
    try {
      wcbtcAddress = wcbtcAddressFor(profile);
    } catch {
      if (profile.name === "test") {
        wcbtcAddress = WCBTC_ADDRESS;
      } else {
        throw new Error(
          `No WCBTC_ADDRESS configured for NETWORK=${profile.name}; pass WCBTC_ADDRESS env`,
        );
      }
    }
  }

  const account =
    profile.name === "dev"
      ? privateKeyToAccount(requireEnv("PRIVATE_KEY") as `0x${string}`)
      : mnemonicToAccount(requireEnv("MNEMONIC"));

  if (profile.name === "main" && amountToWrap > MAINNET_AMOUNT_WARN_CBTC) {
    if (process.env.CONFIRM_LARGE_AMOUNT !== "yes") {
      throw new Error(
        `AMOUNT_CBTC=${formatEther(amountToWrap)} > 0.01 cBTC on mainnet — set CONFIRM_LARGE_AMOUNT=yes if intentional`,
      );
    }
  }

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

  await assertChainId(publicClient, profile.chainId, "wrap-cbtc");

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

  console.log("Network:        ", profile.name);
  console.log("Wallet address: ", account.address);
  console.log("WCBTC token:    ", wcbtcAddress);
  console.log("Amount to wrap: ", formatEther(amountToWrap), "cBTC");
  if (transferTo) {
    console.log(
      `Then transfer:   ${formatEther(transferAmount!)} WCBTC → ${transferTo}`,
    );
  }

  const ethBefore = await publicClient.getBalance({ address: account.address });
  const wcbtcBefore = (await publicClient.readContract({
    address: wcbtcAddress,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [account.address],
  })) as bigint;
  console.log(
    `Before wrap: ${formatEther(ethBefore)} cBTC, WCBTC=${wcbtcBefore}`,
  );

  console.log("\n💸 Wrapping...");
  const wrapHash = await walletClient.writeContract({
    address: wcbtcAddress,
    abi: WCBTC_ABI,
    functionName: "deposit",
    value: amountToWrap,
  });
  const wrapReceipt = await publicClient.waitForTransactionReceipt({
    hash: wrapHash,
  });
  console.log(`✅ Wrap tx ${wrapHash} confirmed in block ${wrapReceipt.blockNumber}`);

  const wcbtcAfter = (await publicClient.readContract({
    address: wcbtcAddress,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [account.address],
  })) as bigint;
  console.log(`WCBTC received: ${wcbtcAfter - wcbtcBefore}`);

  if (transferTo && transferAmount) {
    console.log(`\n📨 Transferring ${transferAmount} WCBTC → ${transferTo}...`);
    const transferHash = await walletClient.writeContract({
      address: wcbtcAddress,
      abi: WCBTC_ABI,
      functionName: "transfer",
      args: [transferTo, transferAmount],
    });
    const transferReceipt = await publicClient.waitForTransactionReceipt({
      hash: transferHash,
    });
    console.log(
      `✅ Transfer tx ${transferHash} confirmed in block ${transferReceipt.blockNumber}`,
    );
  }
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
