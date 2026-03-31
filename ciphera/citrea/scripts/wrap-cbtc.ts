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

// Amount to wrap (0.1 ETH in this example)
const amountToWrap = parseEther("0.1");

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

// Method 1: Using writeContract (recommended)
const wrapTxHash = await senderClient.writeContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "deposit",
    value: amountToWrap,
});

console.log("Wrap transaction hash:", wrapTxHash);

// Wait for transaction confirmation
const wrapReceipt = await publicClient.waitForTransactionReceipt({
    hash: wrapTxHash
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