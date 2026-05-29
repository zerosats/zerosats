// Devnet-only smoke transaction. Refuses any chainId other than 5655.

import {
  createPublicClient,
  createWalletClient,
  http,
  parseEther,
  formatEther,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { assertChainId, citreaDevChain } from "../shared";

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

  await assertChainId(publicClient, DEVNET_CHAIN_ID, "send-devnet-citrea-tx");

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

  console.log(`✅ Chain ID: ${DEVNET_CHAIN_ID}`);
  console.log(`✅ Block:    ${await publicClient.getBlockNumber()}`);

  let balance = await publicClient.getBalance({ address: account.address });
  console.log(`✅ Account:  ${account.address}`);
  console.log(`✅ Balance:  ${formatEther(balance)} cBTC`);

  const gasPrice = await publicClient.getGasPrice();
  console.log(`✅ Gas price: ${gasPrice} wei`);

  console.log("\n💸 Sending test transaction...");
  const hash = await walletClient.sendTransaction({
    to: "0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE",
    value: parseEther("1"),
    gas: 21000n,
    gasPrice,
  });
  console.log(`📝 Transaction hash: ${hash}`);

  const receipt = await publicClient.waitForTransactionReceipt({
    hash,
    timeout: 30000,
  });
  console.log(`✅ Transaction confirmed in block: ${receipt.blockNumber}`);
  console.log(`✅ Gas used: ${receipt.gasUsed}`);
  console.log(`✅ Status:   ${receipt.status}`);

  balance = await publicClient.getBalance({
    address: "0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE",
  });
  console.log(
    `✅ 0xE00fa9663e1060D4a70d2f534ef4Cee477f895dE balance: ${formatEther(balance)} cBTC`,
  );
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
