import { network } from "hardhat";
import { parseEther, encodeFunctionData, decodeFunctionResult } from "viem";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});

console.log("Sending transaction using the OP chain type");

const publicClient = await viem.getPublicClient();
const [senderClient] = await viem.getWalletClients();

const WCBTC_ADDRESS = "0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93";

const WCBTC_ABI = [
    {
        name: "deposit",
        type: "function",
        stateMutability: "payable",
        inputs: [],
        outputs: [],
    },
    {
        name: "withdraw",
        type: "function",
        stateMutability: "nonpayable",
        inputs: [{ name: "wad", type: "uint256" }],
        outputs: [],
    },
    {
        name: "balanceOf",
        type: "function",
        stateMutability: "view",
        inputs: [{ name: "owner", type: "address" }],
        outputs: [{ name: "", type: "uint256" }],
    },
] as const;

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