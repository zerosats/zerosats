import { network } from "hardhat";
import { parseEther } from "viem";
import { WCBTC_ABI, WCBTC_ADDRESS } from "./shared";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});
console.log("Sending transaction using the OP chain type");
const publicClient = await viem.getPublicClient();
const [senderClient] = await viem.getWalletClients();
console.log("Wallet address:", senderClient.account.address);

// Amount to wrap (0.01 ETH in this example)
const amountToWrap = parseEther("0.01");

// Check ETH balance
const ethBalanceBefore = await publicClient.getBalance({
    address: senderClient.account.address,
});
console.log("ETH balance before:", ethBalanceBefore);

// Check WCBTC balance before
const WCBTCBalanceBeforeData = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [senderClient.account.address],
});
console.log("WCBTC balance before:", WCBTCBalanceBeforeData);

// Wrap ETH to WCBTC
console.log(`\nWrapping ${amountToWrap} wei of ETH to WCBTC...`);
const wrapTxHash = await senderClient.writeContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "deposit",
    value: amountToWrap,
});
console.log("Wrap transaction hash:", wrapTxHash);

const wrapReceipt = await publicClient.waitForTransactionReceipt({
    hash: wrapTxHash,
});
console.log("Wrap transaction confirmed in block:", wrapReceipt.blockNumber);

// Check balances after wrapping
const ethBalanceAfter = await publicClient.getBalance({
    address: senderClient.account.address,
});
const WCBTCBalanceAfter = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [senderClient.account.address],
});
console.log("\n=== After Wrapping ===");
console.log("ETH balance after:", ethBalanceAfter);
console.log("WCBTC balance after:", WCBTCBalanceAfter);
console.log("WCBTC received:", WCBTCBalanceAfter - WCBTCBalanceBeforeData);

// ============================================
// Transfer WCBTC to a recipient
// ============================================
const recipientAddress = "0x5b3aa6393f32bc5c1d8086ff4a72cbfd219efbd1"; // <-- replace with the actual recipient
const amountToTransfer = parseEther("0.01");

console.log(`\nTransferring ${amountToTransfer} wei of WCBTC to ${recipientAddress}...`);

// Check recipient balance before
const recipientBalanceBefore = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [recipientAddress],
});
console.log("Recipient WCBTC balance before:", recipientBalanceBefore);

// Execute transfer
const transferTxHash = await senderClient.writeContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "transfer",
    args: [recipientAddress, amountToTransfer],
});
console.log("Transfer transaction hash:", transferTxHash);

// Wait for confirmation
const transferReceipt = await publicClient.waitForTransactionReceipt({
    hash: transferTxHash,
});
console.log("Transfer transaction confirmed in block:", transferReceipt.blockNumber);

// Check balances after transfer
const senderBalanceAfterTransfer = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [senderClient.account.address],
});
const recipientBalanceAfter = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [recipientAddress],
});

console.log("\n=== After Transfer ===");
console.log("Sender WCBTC balance:", senderBalanceAfterTransfer);
console.log("Recipient WCBTC balance:", recipientBalanceAfter);
console.log("Amount transferred:", recipientBalanceAfter - recipientBalanceBefore);