import { network } from "hardhat";

const { viem } = await network.connect({
  network: "citreaTestnet",
  chainId: 5115,
});

console.log("Sending transaction using the OP chain type");

const publicClient = await viem.getPublicClient();
const [senderClient] = await viem.getWalletClients();

console.log("Sending 1 wei from", senderClient.account.address, "to itself");

console.log("Sending L2 transaction");
const tx = await senderClient.sendTransaction({
  to: senderClient.account.address,
  value: 1n,
});

const receipt = await publicClient.waitForTransactionReceipt({ hash: tx });

if (receipt.status !== "success") {
  throw new Error("Transaction reverted");
}
console.log("Transaction sent successfully");
