import rollupV1Artifact from "../artifacts/contracts/rollup2/RollupV1.sol/RollupV1.json";
import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
  encodeFunctionData,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaChain } from "./shared";
import { readFile } from "fs/promises";
import { join } from "path";

const usdcAddress = "0x809d550fca64d94bd9f66e60752a544199cfac3d";
const aggregateVerifierAddr = "0x5eb3bc0a489c5a8288765d2336659ebca68fcd00";
const mintVerifierAddr = "0x36c02da8a0983159322a80ffe9f24b1acff8b570";

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

  const rollupV1 = await walletClient.deployContract({
    abi: rollupV1Artifact.abi,
    bytecode: rollupV1Artifact.bytecode,
    // Add constructor arguments if needed
    // args: [arg1, arg2, ...],
  });

  console.log(`📝 Transaction hash: ${rollupV1}`);

  const rollupV1Addr = (
    await publicClient.waitForTransactionReceipt({ hash: rollupV1 })
  ).contractAddress;

  if (rollupV1Addr === null || rollupV1Addr === undefined)
    throw new Error("Verifier address not found");

  console.log(`✅ Transaction confirmed in block`);

  console.log(`✅ Rollup Contract (Implementation): ${rollupV1Addr}`);

  const emptyMerkleTreeRootHash =
    "0x" +
    (await readFile("contracts/empty_merkle_tree_root_hash.txt"))
      .toString()
      .trimEnd();

  const rollupInitializeCalldata = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName: "initialize",
    args: [
      account.address,
      usdcAddress,
      aggregateVerifierAddr,
      proverAddress,
      validators,
      emptyMerkleTreeRootHash,
    ],
  });

  const rollupProxy = await walletClient.deployContract(
    "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol:TransparentUpgradeableProxy",
    [rollupV1.address, account.address, rollupInitializeCalldata],
    {},
  );

  const proxyAddr = (
    await publicClient.waitForTransactionReceipt({ hash: rollupProxy })
  ).contractAddress;

  console.log(`✅ Rollup Contract (Proxy): ${proxyAddr}`);

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
