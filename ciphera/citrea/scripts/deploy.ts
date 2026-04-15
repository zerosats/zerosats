import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import aliceTokenArtifact from "../artifacts/contracts/helper/AliceERC20.sol/AliceERC20.json";

import proxyArtifact from "../openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol/TransparentUpgradeableProxy.json";

import {
  createPublicClient,
  createWalletClient,
  http,
  formatEther,
  encodeFunctionData,
  getContract,
  formatUnits,
  maxUint256,
  parseAbi,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaDevChain, citreaTestChain } from "./shared";
import IERC20Artifact from "../openzeppelin-contracts/token/ERC20/IERC20.json";

// Auto-updated by generate_fixtures.sh - do not modify manually
const AGG_AGG_VERIFICATION_KEY_HASH = "0x1a2fd848d2ce42026ddbda10d22bbdcad96c89eb501e2c55996c58f76d04840c";

const EIP1967_ADMIN_STORAGE_SLOT =
  "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";

const PROXY_ADMIN_ABI = parseAbi([
  "function owner() view returns (address)",
  "function transferOwnership(address newOwner)",
]);

const ONE_DAY_SECONDS = 86_400n;
const SEVEN_DAYS_SECONDS = 7n * ONE_DAY_SECONDS;
const DEFAULT_PER_MINT_CAP_WEI = 1_000_000_000_000_000n; // 0.001 token
const DEFAULT_GLOBAL_TVL_CAP_WEI = 10_000_000_000_000_000_000n; // 10 token
const SATS_TO_WEI = 10_000_000_000n; // 18-dec BTC wrappers
const DEFAULT_BURN_FEE_WEI = 300n * SATS_TO_WEI; // 300 sats
const MAX_BURN_FEE_WEI = 3_000n * SATS_TO_WEI; // 3000 sats

function parseBigIntEnv(name: string, fallback: bigint): bigint {
  const value = process.env[name];
  if (!value || value.trim() === "") return fallback;
  try {
    return BigInt(value);
  } catch {
    throw new Error(`${name} must be an integer string, got: ${value}`);
  }
}

function parseAddressListEnv(
  name: string,
  fallback: `0x${string}`[],
): `0x${string}`[] {
  const value = process.env[name];
  if (!value || value.trim() === "") return fallback;
  const parsed = value
    .split(",")
    .map((v) => v.trim())
    .filter((v) => v.length > 0) as `0x${string}`[];
  if (parsed.length === 0) {
    throw new Error(`${name} must contain at least one address`);
  }
  return parsed;
}

function readAddressFromSlot(slotValue: `0x${string}` | undefined): `0x${string}` {
  if (!slotValue || slotValue.length < 66) {
    throw new Error(`Unexpected slot value: ${slotValue}`);
  }
  return `0x${slotValue.slice(26)}` as `0x${string}`;
}

async function main() {
  console.log("Initialization...");
  let aggregateVerifierAddr = process.env.VERIFIER;
  const isTestnet = process.env.IS_TESTNET === "1";
  let proverAddress = process.env.PROVER_ADDRESS as `0x${string}`;
  let validators =
    process.env.VALIDATORS?.split(",") ?? ([] as Array<`0x${string}`>);

  console.log("    Citrea Testnet - ", isTestnet);
  console.log("    Prover Address - ", proverAddress);
  console.log("    Validators - ", validators);
  console.log("    Verifier - ", aggregateVerifierAddr);

  const maybeNoopVerifier = (verifier: string) =>
    isTestnet ? verifier : "NoopVerifierHonk.bin";

  let account;
  let rpcUrl;
  let walletClient;

  if (isTestnet) {
    let seed = process.env.MNEMONIC as string;
    account = mnemonicToAccount(seed);
    rpcUrl = "https://rpc.testnet.citrea.xyz";
    if (proverAddress === undefined)
      throw new Error("PROVER_ADDRESS is not set");
    if (validators.length === 0) throw new Error("VALIDATORS is not set");

    walletClient = createWalletClient({
      account,
      chain: {
        ...citreaTestChain,
        rpcUrls: {
          default: { http: [rpcUrl] },
          public: { http: [rpcUrl] },
        },
      },
      transport: http(rpcUrl, {
        timeout: 60000,
        retryCount: 3,
      }),
    });
  } else {
    const privateKey =
      "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    account = privateKeyToAccount(privateKey as `0x${string}`);
    rpcUrl = process.env.TESTING_URL || "http://localhost:12345";

    if (proverAddress === undefined) {
      proverAddress = account.address;
    }

    if (validators.length === 0) {
      validators = [account.address];
    }

    walletClient = createWalletClient({
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
  }

  let ownerAddress = account.address;
  console.log("    Owner - ", ownerAddress);

  // V2 config: secure defaults can be overridden via env vars.
  const v2PerMintCap = parseBigIntEnv(
    "V2_PER_MINT_CAP_WEI",
    DEFAULT_PER_MINT_CAP_WEI,
  );
  const v2GlobalTvlCap = parseBigIntEnv(
    "V2_GLOBAL_TVL_CAP_WEI",
    DEFAULT_GLOBAL_TVL_CAP_WEI,
  );
  const v2OpenProvingDelay = parseBigIntEnv(
    "V2_OPEN_PROVING_DELAY_SECONDS",
    SEVEN_DAYS_SECONDS,
  );
  const v2BurnFee = parseBigIntEnv("V2_BURN_FEE_WEI", DEFAULT_BURN_FEE_WEI);
  const v2FeeSink =
    (process.env.V2_FEE_SINK as `0x${string}` | undefined) ?? ownerAddress;
  const v2TimelockMinDelay = parseBigIntEnv(
    "V2_TIMELOCK_MIN_DELAY_SECONDS",
    ONE_DAY_SECONDS,
  );
  const v2TimelockProposers = parseAddressListEnv(
    "V2_TIMELOCK_PROPOSERS",
    [ownerAddress],
  );
  const v2TimelockExecutors = parseAddressListEnv(
    "V2_TIMELOCK_EXECUTORS",
    [ownerAddress],
  );

  if (v2FeeSink === "0x0000000000000000000000000000000000000000") {
    throw new Error("V2_FEE_SINK cannot be zero address");
  }
  if (v2BurnFee > MAX_BURN_FEE_WEI) {
    throw new Error(
      `V2_BURN_FEE_WEI exceeds max (${MAX_BURN_FEE_WEI.toString()} wei = 3000 sats)`,
    );
  }

  console.log("🚀 Connecting to Citrea...");
  console.log(`    Using URL: ${rpcUrl}`);

  // Create clients with dynamic RPC URL
  const publicClient = createPublicClient({
    chain: {
      ...(isTestnet ? citreaTestChain : citreaDevChain),
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
  console.log(
    `✅ Gas Price: ${gasPrice} wei, ${formatUnits(gasPrice, 9)} GWei`,
  );
  const latestBlock = await publicClient.getBlock({ blockTag: "latest" });
  const baseFee = latestBlock.baseFeePerGas;
  if (!baseFee) {
    throw new Error("Network doesn't support EIP-1559");
  }
  console.log(`✅ Base Fee: ${baseFee}`);

  console.log("\n🎉 Connection successful!");

  let erc20Address;
  let receipt;

  if (isTestnet) {
    erc20Address = "0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93";
    console.log(`✅ Using wrapped cBTC token`);
  } else {
    console.log("\n🔍 Deploying ERC20. Looking for binary file...");

    const erc20Tx = await walletClient.deployContract({
      abi: aliceTokenArtifact.abi,
      bytecode: aliceTokenArtifact.bytecode,
      args: [maxUint256],
    });

    receipt = await publicClient.waitForTransactionReceipt({
      hash: erc20Tx,
    });

    if (receipt.status !== "success") {
      throw new Error("ERC20 deploy reverted");
    }
    console.log(`✅ Transaction confirmed in block`);
    erc20Address = receipt.contractAddress;
    console.log(`✅ ERC20 Deployed`);
  }

  console.log(`✅ ERC20 Contract: ${erc20Address}`);

  if (!aggregateVerifierAddr) {
    console.log("\n🔍 Deploying Verifier. Looking for binary file...");
    aggregateVerifierAddr = await deployBin(
      maybeNoopVerifier("noir/agg_agg_HonkVerifier.bin"),
      publicClient,
      walletClient,
    );
    console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);
  } else {
    console.log(
      `✅ Re-using Aggregate Verifier Contract: ${aggregateVerifierAddr}`,
    );
  }

  console.log("\n🔍 Deploying Rollup");

  const rollupV1 = await walletClient.deployContract({
    abi: rollupV1Artifact.abi,
    bytecode: rollupV1Artifact.bytecode,
  });

  console.log(`📝 Transaction hash: ${rollupV1}`);

  receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupV1,
  });

  if (receipt.status !== "success") {
    throw new Error("RollupV1 implementation deploy reverted");
  }
  console.log(`✅ Transaction confirmed in block`);

  let rollupAddress = receipt.contractAddress;

  console.log(`✅ Rollup Contract (Implementation): ${rollupAddress}`);

  const rollupInitializeCalldata = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName: "initialize",
    args: [
      ownerAddress,
      ownerAddress, // escrowManager must be set later
      erc20Address,
      aggregateVerifierAddr,
      proverAddress,
      validators,
      AGG_AGG_VERIFICATION_KEY_HASH,
    ],
  });

  const rollupProxyTx = await walletClient.deployContract({
    abi: proxyArtifact.abi,
    bytecode: proxyArtifact.bytecode,
    args: [rollupAddress, ownerAddress, rollupInitializeCalldata],
  });

  console.log(`📝 Transaction hash: ${rollupProxyTx}`);

  receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupProxyTx,
  });

  if (receipt.status !== "success") {
    throw new Error("RollupV1 proxy deploy reverted");
  }
  console.log(`✅ Transaction confirmed in block`);
  let rollupProxyAddr = receipt.contractAddress;

  console.log(`✅ Rollup Contract (Proxy): ${rollupProxyAddr}`);

  const adminSlot = await publicClient.getStorageAt({
    address: rollupProxyAddr,
    slot: EIP1967_ADMIN_STORAGE_SLOT,
  });
  const proxyAdminAddress = readAddressFromSlot(adminSlot);
  console.log(`✅ Rollup Proxy Admin: ${proxyAdminAddress}`);

  const rollup = getContract({
    address: rollupProxyAddr,
    abi: rollupV1Artifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });

  const aliceToken = getContract({
    address: erc20Address,
    abi: IERC20Artifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });
  console.log(`✅ Obtained ERC20 contract: ${aliceToken}`);

  console.log("\n🔍 Approving ERC20 spending for proxy...");

  let hash = await aliceToken.write.approve([rollupProxyAddr, maxUint256], {
    gas: 1_000_000n,
  });

  receipt = await publicClient.waitForTransactionReceipt({
    hash: hash,
  });

  if (receipt.status !== "success") {
    throw new Error("ERC20 approve reverted");
  }
  console.log(`✅ Approved maxUint256 to ${rollupProxyAddr}: ${hash}`);

  console.log("\n🔍 Initializing Rollup V2...");
  console.log(
    `    perMintCap=${v2PerMintCap} globalTvlCap=${v2GlobalTvlCap} openProvingDelay=${v2OpenProvingDelay}s burnFee=${v2BurnFee} wei`,
  );
  console.log(`    feeSink=${v2FeeSink}`);
  console.log(
    `    timelockDelay=${v2TimelockMinDelay}s proposers=${v2TimelockProposers.join(",")} executors=${v2TimelockExecutors.join(",")}`,
  );

  hash = await rollup.write.initializeV2(
    [
      v2PerMintCap,
      v2GlobalTvlCap,
      v2OpenProvingDelay,
      v2BurnFee,
      v2FeeSink,
      v2TimelockMinDelay,
      v2TimelockProposers,
      v2TimelockExecutors,
    ],
    {
      gas: 5_000_000n,
    },
  );
  receipt = await publicClient.waitForTransactionReceipt({ hash });
  if (receipt.status !== "success") {
    throw new Error("Rollup V2 initialize reverted");
  }
  console.log(`✅ Rollup V2 initialized: ${hash}`);

  const timelockAddress = (await rollup.read.timelock()) as `0x${string}`;
  const rollupOwner = (await rollup.read.owner()) as `0x${string}`;
  if (rollupOwner.toLowerCase() !== timelockAddress.toLowerCase()) {
    throw new Error(
      `Rollup owner mismatch after V2 init. owner=${rollupOwner} timelock=${timelockAddress}`,
    );
  }
  console.log(`✅ Rollup owner now timelock: ${timelockAddress}`);

  const proxyAdmin = getContract({
    address: proxyAdminAddress,
    abi: PROXY_ADMIN_ABI,
    client: { public: publicClient, wallet: walletClient },
  });

  const proxyAdminOwnerBefore =
    (await proxyAdmin.read.owner()) as `0x${string}`;
  console.log(`✅ ProxyAdmin owner before handoff: ${proxyAdminOwnerBefore}`);

  if (proxyAdminOwnerBefore.toLowerCase() !== timelockAddress.toLowerCase()) {
    if (proxyAdminOwnerBefore.toLowerCase() !== ownerAddress.toLowerCase()) {
      throw new Error(
        `Unexpected ProxyAdmin owner ${proxyAdminOwnerBefore}; expected ${ownerAddress} before transfer`,
      );
    }
    const transferHash = await proxyAdmin.write.transferOwnership(
      [timelockAddress],
      { account },
    );
    const transferReceipt = await publicClient.waitForTransactionReceipt({
      hash: transferHash,
    });
    if (transferReceipt.status !== "success") {
      throw new Error("ProxyAdmin transferOwnership reverted");
    }
    console.log(`✅ ProxyAdmin ownership transferred to timelock: ${transferHash}`);
  } else {
    console.log("✅ ProxyAdmin ownership already timelocked");
  }

  const proxyAdminOwnerAfter = (await proxyAdmin.read.owner()) as `0x${string}`;
  if (proxyAdminOwnerAfter.toLowerCase() !== timelockAddress.toLowerCase()) {
    throw new Error(
      `ProxyAdmin owner mismatch after transfer. owner=${proxyAdminOwnerAfter} timelock=${timelockAddress}`,
    );
  }
  console.log(`✅ ProxyAdmin owner now timelock: ${proxyAdminOwnerAfter}`);

  // Machine-readable output for the test harness
  console.log(
    `DEPLOY_OUTPUT=${JSON.stringify({
      rollupProxy: rollupProxyAddr,
      rollupImplementation: rollupAddress,
      rollupOwner,
      timelock: timelockAddress,
      proxyAdmin: proxyAdminAddress,
      proxyAdminOwner: proxyAdminOwnerAfter,
      perMintCap: v2PerMintCap.toString(),
      globalTvlCap: v2GlobalTvlCap.toString(),
      openProvingDelaySeconds: v2OpenProvingDelay.toString(),
      burnFeeWei: v2BurnFee.toString(),
      feeSink: v2FeeSink,
      erc20: erc20Address,
      verifier: aggregateVerifierAddr,
    })}`,
  );

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
