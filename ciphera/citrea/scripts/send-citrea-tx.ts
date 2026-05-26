// Smoke transaction: sends 1 wei to self on the configured network.
// Mostly useful for confirming RPC reachability and key configuration.

import { createPublicClient, createWalletClient, http } from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  parseNetwork,
  requireEnv,
  resolveRpcUrl,
} from "./shared";

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);

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

  await assertChainId(publicClient, profile.chainId, "send-citrea-tx");

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

  console.log(`Network: ${profile.name} (chainId=${profile.chainId})`);
  console.log(`Sending 1 wei from ${account.address} to itself`);

  const tx = await walletClient.sendTransaction({
    to: account.address,
    value: 1n,
  });

  const receipt = await publicClient.waitForTransactionReceipt({ hash: tx });
  if (receipt.status !== "success") {
    throw new Error("Transaction reverted");
  }
  console.log(`✅ Confirmed in block ${receipt.blockNumber}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
