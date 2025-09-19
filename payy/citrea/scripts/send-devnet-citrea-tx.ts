import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";

// Simple custom chain definition for Citrea
const citreaChain = {
  id: 5655,
  name: "Citrea Devnet",
  network: "citreaDevnet",
  nativeCurrency: {
    decimals: 18,
    name: "Citrea Bitcoin",
    symbol: "cBTC",
  },
  rpcUrls: {
    default: { http: [""] }, // Will be set dynamically
    public: { http: [""] },
  },
} as const;

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
  //const account = mnemonicToAccount('rail flame music embark label blade bomb front reform mango aisle moment')

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

  // Test basic connectivity
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

  // Get gas price
  const gasPrice = await publicClient.getGasPrice();
  console.log(`✅ Gas Price: ${gasPrice} wei`);

  // Example transaction (uncomment to test)

  console.log("\n💸 Sending test transaction...");
  const hash = await walletClient.sendTransaction({
    to: "0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE", // Second hardhat account
    value: parseEther("1"),
    gas: 21000n,
    gasPrice: gasPrice,
  });

  console.log(`📝 Transaction hash: ${hash}`);

  const receipt = await publicClient.waitForTransactionReceipt({
    hash,
    timeout: 30000,
  });

  console.log(`✅ Transaction confirmed in block: ${receipt.blockNumber}`);
  console.log(`✅ Gas used: ${receipt.gasUsed}`);
  console.log(`✅ Status: ${receipt.status}`);

  balance = await publicClient.getBalance({
    address: "0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE",
  });
  console.log(`✅ Account: 0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE`);
  console.log(`✅ Balance: ${formatEther(balance)} cBTC`);

  console.log("\n🎉 Connection successful!");
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
