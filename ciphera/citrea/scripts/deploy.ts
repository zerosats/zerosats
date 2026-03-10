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
  parseUnits,
  formatUnits,
  maxUint256,
} from "viem";
import { privateKeyToAccount, mnemonicToAccount } from "viem/accounts";
import { deployBin, citreaDevChain, citreaTestChain } from "./shared";
import { readFile } from "fs/promises";
import { join } from "path";
import IERC20Artifact from "../openzeppelin-contracts/token/ERC20/IERC20.json";

// Auto-updated by generate_fixturecs.sh - do not modify manually
const AGG_AGG_VERIFICATION_KEY_HASH =
  "0x1594fce0e59bc3785292f9ab4f5a1e45f5795b4a616aff5cdc4d32a223f69f0c";

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

  console.log("🚀 Connecting to Citrea...");
  console.log(`    Using URL: ${rpcUrl}`);

  // Create clients with dynamic RPC URL
  const publicClient = createPublicClient({
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
    `✅ Gas Price: ${gasPrice} wei, ${formatUnits(gasPrice, "gwei")} GWei`,
  );
  const latestBlock = await publicClient.getBlock("latest");
  const baseFee = latestBlock.baseFeePerGas;
  if (!baseFee) {
    throw new Error("Network doesn't support EIP-1559");
  }
  console.log(`✅ Base Fee: ${baseFee}`);

  console.log("\n🎉 Connection successful!");

  let erc20Address;
  let receipt;

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

    if (receipt.status == "success") {
      console.log(`✅ Transaction confirmed in block`);
    } else {
      console.log(`❌ Transaction reverted`);
    }
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

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  let rollupAddress = receipt.contractAddress;

  console.log(`✅ Rollup Contract (Implementation): ${rollupAddress}`);

  const rollupInitializeCalldata = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName: "initialize",
    args: [
      ownerAddress,
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

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }
  let rollupProxyAddr = receipt.contractAddress;

  console.log(`✅ Rollup Contract (Proxy): ${rollupProxyAddr}`);

  const eip1967AdminStorageSlot =
    "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";
  let admin = await publicClient.getStorageAt({
    address: rollupProxyAddr,
    slot: eip1967AdminStorageSlot,
  });
  admin = `0x${admin?.slice(2 + 12 * 2)}`;
  console.log(`✅ Rollup Proxy Admin: ${admin}`);

  /*
    const proxyAdmin = await getContract({
        address: admin,
        abi: "@openzeppelin/contracts/proxy/transparent/ProxyAdmin.sol:ProxyAdmin",
        client: {public: publicClient, wallet: walletClient},
    });
*/

  /*
    console.log("\n🔍 Sending some tokens to prover");
    const sendTx = await walletClient.sendTransaction({
        to: proverAddress,
       value: 1n,
    });
    await publicClient.waitForTransactionReceipt({ hash: sendTx });
    console.log("Transaction sent successfully");
*/

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

  if (receipt.status == "success") {
    console.log(`✅ Approved maxUint256 to ${rollupProxyAddr}: ${hash}`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  // Register the mock BTC note kind used by the Rust test suite.
  // RollupV1.initialize() only registers the Citrea testnet note kind (chain 5115),
  // but Note::new_with_psi() in the Rust code produces a different note kind.
  if (!isTestnet) {
    console.log("\n🔍 Registering mock BTC note kind for dev testing...");
    const rollupProxy = getContract({
      address: rollupProxyAddr,
      abi: rollupV1Artifact.abi,
      client: { public: publicClient, wallet: walletClient },
    });

    // Note kind produced by Note::new_with_psi() — see bridged_polygon_usdc_note_kind() in util.rs
    // TODO: Go over polygon notes later
    const mockBtcNoteKind =
      "0x000200000000000000893c499c542cef5e3811e1192ce70d8cc03d5c33590000";

    hash = await rollupProxy.write.addToken([mockBtcNoteKind, erc20Address], {
      gas: 1_000_000n,
    });

    receipt = await publicClient.waitForTransactionReceipt({ hash });
    if (receipt.status !== "success") {
      throw new Error(
        `Failed to register mock BTC note kind for ${erc20Address}`,
      );
    }
    console.log(`✅ Registered mock BTC note kind → ${erc20Address}`);
  }

  // Machine-readable output for the test harness
  console.log(
    `DEPLOY_OUTPUT=${JSON.stringify({
      rollupProxy: rollupProxyAddr,
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
