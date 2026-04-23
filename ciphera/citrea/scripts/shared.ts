import {
  encodeAbiParameters,
  getContract,
  keccak256,
  parseGwei,
  zeroHash,
} from "viem";
import type { PublicClient, WalletClient } from "viem";
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

export const WCBTC_ADDRESS = "0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93" as const;

export const WCBTC_ABI = [
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

export function linkBin(
  bin: string,
  links: Record<string, `0x${string}`>,
): string {
  for (const [placeholder, address] of Object.entries(links)) {
    const addr = address.slice(2).toLowerCase().padStart(40, "0");
    bin = bin.split(placeholder).join(addr);
  }
  return bin;
}

export const TIMELOCK_ABI = [
  {
    name: "getMinDelay",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ type: "uint256" }],
  },
  {
    name: "hashOperation",
    type: "function",
    stateMutability: "pure",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
    ],
    outputs: [{ type: "bytes32" }],
  },
  {
    name: "schedule",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
      { name: "delay", type: "uint256" },
    ],
    outputs: [],
  },
  {
    name: "execute",
    type: "function",
    stateMutability: "payable",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
    ],
    outputs: [],
  },
  {
    name: "isOperationPending",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "bool" }],
  },
  {
    name: "isOperationReady",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "bool" }],
  },
  {
    name: "isOperationDone",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "bool" }],
  },
  {
    name: "getTimestamp",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "uint256" }],
  },
  {
    name: "PROPOSER_ROLE",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ type: "bytes32" }],
  },
  {
    name: "EXECUTOR_ROLE",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ type: "bytes32" }],
  },
  {
    name: "hasRole",
    type: "function",
    stateMutability: "view",
    inputs: [
      { name: "role", type: "bytes32" },
      { name: "account", type: "address" },
    ],
    outputs: [{ type: "bool" }],
  },
] as const;

export type TimelockMode = "schedule" | "execute" | "auto";

export function parseTimelockMode(raw: string | undefined): TimelockMode {
  const mode = (raw || "auto").toLowerCase();
  if (mode !== "schedule" && mode !== "execute" && mode !== "auto") {
    throw new Error(
      `MODE must be 'schedule', 'execute', or 'auto' (got '${raw}')`,
    );
  }
  return mode;
}

export async function timelockDispatch(params: {
  publicClient: PublicClient;
  walletClient: WalletClient;
  timelock: `0x${string}`;
  target: `0x${string}`;
  data: `0x${string}`;
  mode: TimelockMode;
  value?: bigint;
  salt?: `0x${string}`;
  predecessor?: `0x${string}`;
  pollIntervalMs?: number;
}): Promise<{
  opId: `0x${string}`;
  salt: `0x${string}`;
  predecessor: `0x${string}`;
}> {
  const {
    publicClient,
    walletClient,
    timelock,
    target,
    data,
    mode,
    value = 0n,
    predecessor = zeroHash,
    pollIntervalMs = 10_000,
  } = params;

  // Deterministic default so retries converge on the same operation id.
  const salt =
    params.salt ??
    keccak256(
      encodeAbiParameters(
        [{ type: "address" }, { type: "bytes" }],
        [target, data],
      ),
    );

  const tl = getContract({
    address: timelock,
    abi: TIMELOCK_ABI,
    client: { public: publicClient, wallet: walletClient },
  });

  const opId = (await tl.read.hashOperation([
    target,
    value,
    data,
    predecessor,
    salt,
  ])) as `0x${string}`;

  console.log(`🕒 Timelock:     ${timelock}`);
  console.log(`🎯 Target:       ${target}`);
  console.log(`🧂 Salt:         ${salt}`);
  console.log(`🔑 Operation id: ${opId}`);

  const caller = walletClient.account?.address;
  if (caller) {
    const [proposerRole, executorRole] = await Promise.all([
      tl.read.PROPOSER_ROLE(),
      tl.read.EXECUTOR_ROLE(),
    ]);
    const [isProposer, isExecutor, anyoneCanExecute] = await Promise.all([
      tl.read.hasRole([proposerRole as `0x${string}`, caller]),
      tl.read.hasRole([executorRole as `0x${string}`, caller]),
      tl.read.hasRole([
        executorRole as `0x${string}`,
        "0x0000000000000000000000000000000000000000",
      ]),
    ]);
    console.log(
      `👤 Caller roles: proposer=${isProposer} executor=${isExecutor} (open-execute=${anyoneCanExecute})`,
    );
    if ((mode === "schedule" || mode === "auto") && !isProposer) {
      console.warn(
        "⚠️  Caller lacks PROPOSER_ROLE — schedule() will revert.",
      );
    }
    if (
      (mode === "execute" || mode === "auto") &&
      !isExecutor &&
      !anyoneCanExecute
    ) {
      console.warn(
        "⚠️  Caller lacks EXECUTOR_ROLE and open-execute is off — execute() will revert.",
      );
    }
  }

  if ((await tl.read.isOperationDone([opId])) as boolean) {
    console.log("✅ Operation already executed — nothing to do");
    return { opId, salt, predecessor };
  }

  let pending = (await tl.read.isOperationPending([opId])) as boolean;

  if (mode === "schedule" || mode === "auto") {
    if (pending) {
      console.log("ℹ️  Operation already scheduled — skipping schedule step");
    } else {
      const delay = (await tl.read.getMinDelay()) as bigint;
      console.log(`⏳ Scheduling with delay: ${delay}s`);
      const scheduleHash = await tl.write.schedule(
        [target, value, data, predecessor, salt, delay],
        { gas: 300_000n },
      );
      console.log(`📝 schedule tx: ${scheduleHash}`);
      const receipt = await publicClient.waitForTransactionReceipt({
        hash: scheduleHash,
      });
      if (receipt.status !== "success") {
        throw new Error("Timelock schedule() transaction reverted");
      }
      const readyTs = (await tl.read.getTimestamp([opId])) as bigint;
      console.log(
        `✅ Scheduled. Ready at: ${new Date(Number(readyTs) * 1000).toISOString()}`,
      );
      pending = true;
    }
    if (mode === "schedule") return { opId, salt, predecessor };
  }

  // execute or auto
  if (!pending) {
    throw new Error(
      `Operation ${opId} is not scheduled. Run with MODE=schedule first.`,
    );
  }

  while (!((await tl.read.isOperationReady([opId])) as boolean)) {
    const ts = (await tl.read.getTimestamp([opId])) as bigint;
    const now = BigInt(Math.floor(Date.now() / 1000));
    const waitSec = ts > now ? ts - now : 0n;
    if (mode === "execute") {
      throw new Error(
        `Operation not ready. Ready in ${waitSec}s at ${new Date(
          Number(ts) * 1000,
        ).toISOString()}`,
      );
    }
    console.log(`⏱️  Not ready yet. ${waitSec}s remaining...`);
    await new Promise((r) => setTimeout(r, pollIntervalMs));
  }

  console.log("🚀 Executing...");
  const executeHash = await tl.write.execute(
    [target, value, data, predecessor, salt],
    { gas: 500_000n, value },
  );
  console.log(`📝 execute tx: ${executeHash}`);
  const receipt = await publicClient.waitForTransactionReceipt({
    hash: executeHash,
  });
  if (receipt.status !== "success") {
    throw new Error("Timelock execute() transaction reverted");
  }
  console.log(`✅ Executed in block ${receipt.blockNumber}`);

  return { opId, salt, predecessor };
}

export async function deployBin(
  binFile: string,
  publicClient: PublicClient,
  walletClient: WalletClient,
  links?: Record<string, `0x${string}`>,
): Promise<`0x${string}`> {
  let bin = (await readFile(`contracts/${binFile}`)).toString().trimEnd();
  if (links) bin = linkBin(bin, links);

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

  if (receipt.status !== "success") {
    throw new Error(`Deploy of ${binFile} reverted`);
  }
  console.log(`✅ Transaction confirmed in block`);

  if (!receipt.contractAddress) {
    throw new Error(`Deploy of ${binFile} succeeded but no contract address in receipt`);
  }
  return receipt.contractAddress;
}
