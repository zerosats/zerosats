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

// Optional: Unwrap WCBTC back to ETH
console.log("\n=== Unwrapping WCBTC back to ETH ===");
const amountToUnwrap = parseEther("0.0001"); // Unwrap half

const unwrapTxHash = await senderClient.writeContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "withdraw",
    args: [amountToUnwrap],
});

console.log("Unwrap transaction hash:", unwrapTxHash);

const unwrapReceipt = await publicClient.waitForTransactionReceipt({
    hash: unwrapTxHash
});
console.log("Unwrap transaction confirmed in block:", unwrapReceipt.blockNumber);

// Final balance check
const finalEthBalance = await publicClient.getBalance({
    address: senderClient.account.address,
});
const finalWCBTCBalance = await publicClient.readContract({
    address: WCBTC_ADDRESS,
    abi: WCBTC_ABI,
    functionName: "balanceOf",
    args: [senderClient.account.address],
});

console.log("\n=== After (Un)-Wrapping ===\n");
console.log("ETH balance:", finalEthBalance);
console.log("WCBTC balance:", finalWCBTCBalance);