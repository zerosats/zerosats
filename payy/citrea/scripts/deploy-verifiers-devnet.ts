import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaChain } from "./shared";
import { readFile } from "fs/promises";

async function main() {
  console.log("🚀 Connecting to Citrea...");

  // Auto-detect environment and set URL
  const rpcUrl = "http://localhost:12345";
  console.log(`RPC URL: ${rpcUrl}`);

  // Create clients with dynamic RPC URL
  const publicClient = createPublicClient({
    chain: {
      ...citreaChain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, {
      timeout: 30000,
      retryCount: 3,
    }),
  });

  const privateKey =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
  const account = privateKeyToAccount(privateKey as `0x${string}`);

  const walletClient = createWalletClient({
    account,
    chain: {
      ...citreaChain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, {
      timeout: 30000,
      retryCount: 3,
    }),
  });

  console.log("\n🔍 Testing connection...");
  const chainId = await publicClient.getChainId();
  console.log(`✅ Chain ID: ${chainId}`);

  const blockNumber = await publicClient.getBlockNumber();
  console.log(`✅ Block Number: ${blockNumber}`);

  // Check account balance
  let balance = await publicClient.getBalance({
    address: account.address,
  });
  console.log(`✅ Account: ${account.address}`);
  console.log(`✅ Balance: ${formatEther(balance)} cBTC`);

  console.log("\n🔍 Looking for binary files and deploying contracts...");

  const aggregateVerifierAddr = await deployBin(
    "noir/agg_agg_HonkVerifier.bin",
    publicClient,
    walletClient,
  );

  console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);
  /*
  const mintVerifierAddr = await deployBin(
    "noir/mint_HonkVerifier.bin",
    publicClient,
    walletClient,
  );

  console.log(`✅ Mint Verifier Contract: ${mintVerifierAddr}`);
  */
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
