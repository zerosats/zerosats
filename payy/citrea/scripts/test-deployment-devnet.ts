import IUSDCArtifact from "../artifacts/contracts/IUSDC.sol/IUSDC.json";
import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
  encodeFunctionData,
  getContract,
  parseUnits,
  maxUint256,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaChain } from "./shared";
import { readFile } from "fs/promises";
import { join } from "path";

const usdcAddress = "0x809d550fca64d94bd9f66e60752a544199cfac3d";
const aggregateVerifierAddr = "0x5eb3bc0a489c5a8288765d2336659ebca68fcd00";
const mintVerifierAddr = "0x36c02da8a0983159322a80ffe9f24b1acff8b570";
const rollupImplAddr = "0xfd471836031dc5108809d173a067e8486b9047a3";
const rollupProxyAddr = "0xcbeaf3bde82155f56486fb5a1072cb8baaf547cc";

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

  const proverAddress = account.address;
  const validators = [account.address];

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
  console.log("\n🎉 Connection successful!");

  console.log("\n🔍 Testing deployment...");
  const usdc = getContract({
    address: usdcAddress,
    abi: IUSDCArtifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });
  console.log(`✅ USDC contract: ${usdcAddress}`);
  let hash = await usdc.write.initialize(
    [
      "USD Coin",
      "USDC",
      "USD",
      6,
      account.address,
      account.address,
      account.address,
      account.address,
    ],
    {
      gas: 1_000_000n,
    },
  );
  await publicClient.waitForTransactionReceipt({ hash });
  console.log(`✅ Sent test USDC: ${hash}`);

  hash = await usdc.write.initializeV2(["USD Coin"], {
    gas: 1_000_000n,
  });
  await publicClient.waitForTransactionReceipt({ hash });

  console.log(`✅ V2 initialized: ${hash}`);

  hash = await usdc.write.initializeV2_1([account.address], {
    gas: 1_000_000n,
  });
  await publicClient.waitForTransactionReceipt({ hash });

  console.log(`✅ V2.1 initialized: ${hash}`);

  hash = await usdc.write.configureMinter(
    [account.address, parseUnits("1000000000", 6)],
    {
      gas: 1_000_000n,
    },
  );
  await publicClient.waitForTransactionReceipt({ hash });

  console.log(`✅ Minter configured: ${hash}`);

  hash = await usdc.write.mint([
    account.address,
    parseUnits("1000000000", 6),
  ], {
    gas: 1_000_000n,
  });
  await publicClient.waitForTransactionReceipt({ hash });

  console.log(`✅ Minted to ${account.address}: ${hash}`);

  hash = await usdc.write.approve([rollupProxyAddr, maxUint256], {
    gas: 1_000_000n,
  });
  await publicClient.waitForTransactionReceipt({ hash });

  console.error("All transactions executed");
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
