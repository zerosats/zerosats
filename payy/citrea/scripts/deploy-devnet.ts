import rollupV1Artifact from "../artifacts/contracts/rollup2/RollupV1.sol/RollupV1.json";
import proxyArtifact from "../openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol/TransparentUpgradeableProxy.json";

import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
  encodeFunctionData,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaDevChain } from "./shared";
import { readFile } from "fs/promises";
import { join } from "path";

// Auto-updated by generate_fixturecs.sh - do not modify manually
const AGG_AGG_VERIFICATION_KEY_HASH =
  "0x1594fce0e59bc3785292f9ab4f5a1e45f5795b4a616aff5cdc4d32a223f69f0c";

const usdcAddress = "0x5fbdb2315678afecb367f032d93f642f64180aa3";
const aggregateVerifierAddr = "0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9";

async function main() {
  console.log("🚀 Connecting to Citrea...");

  // Auto-detect environment and set URL
  const rpcUrl = "http://localhost:12345";
  console.log(`RPC URL: ${rpcUrl}`);

  // Create clients with dynamic RPC URL
  const publicClient = createPublicClient({
    chain: {
      ...citreaDevChain,
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
      ...citreaDevChain,
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

  const rollupV1 = await walletClient.deployContract({
    abi: rollupV1Artifact.abi,
    bytecode: rollupV1Artifact.bytecode,
  });

  console.log(`📝 Transaction hash: ${rollupV1}`);

  let receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupV1,
  });

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  let rollupAddress = receipt.contractAddress;

  console.log(`✅ Rollup Contract (Implementation): ${rollupAddress}`);

  const rollupInitializeCalldata = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName: "initialize",
    args: [
      account.address,
      usdcAddress,
      aggregateVerifierAddr,
      proverAddress,
      validators,
      AGG_AGG_VERIFICATION_KEY_HASH,
    ],
  });

  const rollupProxyTx = await walletClient.deployContract({
    abi: proxyArtifact.abi,
    bytecode: proxyArtifact.bytecode,
    args: [rollupAddress, account.address, rollupInitializeCalldata],
  });

  console.log(`📝 Transaction hash: ${rollupProxyTx}`);

  receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupProxyTx,
  });

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  console.log(`✅ Rollup Contract (Proxy): ${receipt.contractAddress}`);

  /*
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
    */
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
