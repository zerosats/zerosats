// Unwrap WCBTC → native cBTC. Operator convenience tool.

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
    name: "withdraw",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [{ name: "wad", type: "uint256" }],
    outputs: [],
  },
  {
    name: "balanceOf",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "owner", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
] as const;

const MAINNET_AMOUNT_WARN_CBTC = parseEther("0.01");

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);
  const amountToUnwrap = parseEther(requireEnv("AMOUNT_CBTC"));

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

  if (profile.name === "main" && amountToUnwrap > MAINNET_AMOUNT_WARN_CBTC) {
    if (process.env.CONFIRM_LARGE_AMOUNT !== "yes") {
      throw new Error(
        `AMOUNT_CBTC=${formatEther(amountToUnwrap)} > 0.01 cBTC on mainnet — set CONFIRM_LARGE_AMOUNT=yes if intentional`,
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

  await assertChainId(publicClient, profile.chainId, "unwrap-cbtc");

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

  console.log("Network:          ", profile.name);
  console.log("Wallet address:   ", account.address);
  console.log("WCBTC token:      ", wcbtcAddress);
  console.log("Amount to unwrap: ", formatEther(amountToUnwrap), "cBTC");

  const unwrapHash = await walletClient.writeContract({
    address: wcbtcAddress,
    abi: WCBTC_ABI,
    functionName: "withdraw",
    args: [amountToUnwrap],
  });
  const receipt = await publicClient.waitForTransactionReceipt({
    hash: unwrapHash,
  });
  console.log(`✅ Unwrap tx ${unwrapHash} confirmed in block ${receipt.blockNumber}`);

  const ethBalance = await publicClient.getBalance({ address: account.address });
  const wcbtcBalance = (await publicClient.readContract({
    address: wcbtcAddress,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [account.address],
  })) as bigint;
  console.log(`After: cBTC=${formatEther(ethBalance)}, WCBTC=${wcbtcBalance}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
