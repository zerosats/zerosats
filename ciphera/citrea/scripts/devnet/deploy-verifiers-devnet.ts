// Devnet-only: deploys the aggregate Honk verifier binary against the local
// Citrea regtest node. Refuses to run against any chain other than 5655.

import {
  createPublicClient,
  createWalletClient,
  http,
  formatEther,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { assertChainId, citreaDevChain, deployBin } from "../shared";

const DEVNET_CHAIN_ID = 5655;

async function main() {
  const rpcUrl =
    process.env.RPC_URL || process.env.TESTING_URL || "http://localhost:12345";
  console.log("🚀 Connecting to Citrea devnet...");
  console.log(`RPC URL: ${rpcUrl}`);

  const publicClient = createPublicClient({
    chain: {
      ...citreaDevChain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 30000, retryCount: 3 }),
  });

  await assertChainId(
    publicClient,
    DEVNET_CHAIN_ID,
    "deploy-verifiers-devnet",
  );

  const privateKey = (process.env.PRIVATE_KEY ||
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80") as `0x${string}`;
  const account = privateKeyToAccount(privateKey);

  const walletClient = createWalletClient({
    account,
    chain: {
      ...citreaDevChain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 30000, retryCount: 3 }),
  });

  const blockNumber = await publicClient.getBlockNumber();
  console.log(`✅ Chain ID: ${DEVNET_CHAIN_ID}`);
  console.log(`✅ Block:    ${blockNumber}`);

  const balance = await publicClient.getBalance({ address: account.address });
  console.log(`✅ Account:  ${account.address}`);
  console.log(`✅ Balance:  ${formatEther(balance)} cBTC`);

  console.log("\n🔍 Deploying aggregate verifier...");
  const aggregateVerifierAddr = await deployBin(
    "noir/agg_agg_HonkVerifier.bin",
    publicClient,
    walletClient,
  );
  console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
