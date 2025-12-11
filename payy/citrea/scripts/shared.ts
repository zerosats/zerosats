import { parseGwei, serializeTransaction } from "viem";
import { readFile } from "fs/promises";

// Simple custom chain definition for Citrea local regtest configuration
export const citreaDevChain = {
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
  // Add default gas configuration
  fees: {
    defaultPriorityFee: parseGwei("10"),
    baseFeeMultiplier: 10,
  },
} as const;

export const citreaTestChain = {
  id: 5115,
  name: "Citrea Testnet",
  network: "citreaTestnet",
  nativeCurrency: {
    decimals: 18,
    name: "Citrea Bitcoin",
    symbol: "cBTC",
  },
  rpcUrls: {
    default: { http: [""] }, // Will be set dynamically
    public: { http: [""] },
  },
  // Add default gas configuration
  fees: {
    defaultPriorityFee: parseGwei("0.5"),
    baseFeeMultiplier: 1.1,
  },
} as const;

export async function deployBin(
  binFile: string,
  publicClient: any,
  walletClient: any,
): Promise<`0x${string}`> {
  const bin = (await readFile(`contracts/${binFile}`)).toString().trimEnd();

  console.log("\n💸 Sending deploy transaction...");
  console.log(
    "Deploying ",
    binFile,
    " contract of size: ",
    bin.length / 2,
    "bytes",
  );

  const verifierTx = await walletClient.deployContract({
    bytecode: `0x${bin}`,
    abi: [],
    gas: 8000000n,
    // maxFeePerGas: parseGwei('100000'), // Increase this
    // maxPriorityFeePerGas: parseGwei('100'), // Increase this
  });

  console.log(`📝 Transaction hash: ${verifierTx}`);

  const receipt = await publicClient.waitForTransactionReceipt({
    hash: verifierTx,
  });

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  return receipt.contractAddress;
}
