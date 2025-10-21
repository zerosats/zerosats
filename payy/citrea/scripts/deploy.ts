import rollupV1Artifact from "../artifacts/contracts/rollup2/RollupV1.sol/RollupV1.json";
import proxyArtifact
    from "../openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol/TransparentUpgradeableProxy.json";

import {
    createPublicClient,
    createWalletClient,
    http,
    parseEther,
    formatEther,
    encodeFunctionData, getContract, parseUnits, maxUint256,
} from "viem";
import {privateKeyToAccount, mnemonicToAccount} from "viem/accounts";
import {deployBin, citreaChain} from "./shared";
import {readFile} from "fs/promises";
import {join} from "path";
import IUSDCArtifact from "../artifacts/contracts/IUSDC.sol/IUSDC.json";

// Auto-updated by generate_fixturecs.sh - do not modify manually
const AGG_AGG_VERIFICATION_KEY_HASH =
    "0x1594fce0e59bc3785292f9ab4f5a1e45f5795b4a616aff5cdc4d32a223f69f0c";

const USDC_ADDRESSES: Record<string, string> = {
    // Ethereum Mainnet
    1: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    // Ethereum Goerli Testnet
    // 5: '0x07865c6e87b9f70255377e024ace6630c1eaa37f',
    // Polygon Mainnet
    137: "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
    // Polygon Mumbai Testnet
    // 80001: '0x2058A9D7613eEE744279e3856Ef0eAda5FCbaA7e'
};

const aggregateVerifierAddr = "0xe7f1725e7734ce288f8367e1bb143e90bb3f0512";

async function main() {
    console.log("🚀 Connecting to Citrea...");

    // Auto-detect environment and set URL
    const rpcUrl = "http://localhost:12345";
    console.log(`RPC URL: ${rpcUrl}`);

    // Create clients with dynamic RPC URL
    const publicClient = createPublicClient({
        chain: {
            ...citreaChain,
            rpcUrls: {
                default: {http: [rpcUrl]},
                public: {http: [rpcUrl]},
            },
        },
        transport: http(rpcUrl, {
            timeout: 30000,
            retryCount: 3,
        }),
    });

    const privateKey =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const account = privateKeyToAccount(privateKey as `0x${string}`);
    //const account = mnemonicToAccount('rail flame music embark label blade bomb front reform mango aisle moment')

    const walletClient = createWalletClient({
        account,
        chain: {
            ...citreaChain,
            rpcUrls: {
                default: {http: [rpcUrl]},
                public: {http: [rpcUrl]},
            },
        },
        transport: http(rpcUrl, {
            timeout: 30000,
            retryCount: 3,
        }),
    });

    const proverAddress = account.address;
    const validators = [account.address];

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
    console.log(`✅ Gas Price: ${gasPrice} wei`);
    console.log("\n🎉 Connection successful!");

    console.log("\n🔍 Deploying USDC. Looking for binary file...");

    const usdcAddress = await deployBin(
        "USDC.bin",
        publicClient,
        walletClient,
    );
    console.log(`✅ USDC Contract: ${usdcAddress}`);

    console.log("\n🔍 Deploying Verifier. Looking for binary file...");

    const aggregateVerifierAddr = await deployBin(
        "noir/agg_agg_HonkVerifier.bin",
        publicClient,
        walletClient,
    );

    console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);

    console.log("\n🔍 Deploying Rollup");

    const rollupV1 = await walletClient.deployContract({
        abi: rollupV1Artifact.abi,
        bytecode: rollupV1Artifact.bytecode,
    });

    console.log(`📝 Transaction hash: ${rollupV1}`);

    let receipt = await publicClient.waitForTransactionReceipt({
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
            account.address,
            usdcAddress,
            aggregateVerifierAddr,
            proverAddress,
            validators,
            AGG_AGG_VERIFICATION_KEY_HASH,
        ],
    });

    const rollupProxyTx = await walletClient.deployContract({
        abi: proxyArtifact.abi,
        bytecode: proxyArtifact.bytecode,
        args: [rollupAddress, account.address, rollupInitializeCalldata],
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

    //console.log("\n🔍 Sending some tokens to prover");
    //const sendTx = await walletClient.sendTransaction({
    //    to: proverAddress,
    //   value: 1n,
    //});
    //await publicClient.waitForTransactionReceipt({ hash: sendTx });
    //console.log("Transaction sent successfully");

    console.log("\n🔍 Testing deployment...");

    const usdc = getContract({
        address: usdcAddress,
        abi: IUSDCArtifact.abi,
        client: {public: publicClient, wallet: walletClient},
    });

    console.log(`✅ Obtained USDC contract: ${usdcAddress}`);

    let hash = await usdc.write.initialize(
        [
            "USD Coin",
            "USDC",
            "USD",
            6,
            account.address,
            account.address,
            account.address,
            account.address,
        ],
        {
            gas: 1_000_000n,
        },
    );
    await publicClient.waitForTransactionReceipt({hash});
    console.log(`✅ Sent test USDC: ${hash}`);

    hash = await usdc.write.initializeV2(["USD Coin"], {
        gas: 1_000_000n,
    });
    await publicClient.waitForTransactionReceipt({hash});

    console.log(`✅ V2 initialized: ${hash}`);

    hash = await usdc.write.initializeV2_1([account.address], {
        gas: 1_000_000n,
    });
    await publicClient.waitForTransactionReceipt({hash});

    console.log(`✅ V2.1 initialized: ${hash}`);

    hash = await usdc.write.configureMinter(
        [account.address, parseUnits("1000000000", 6)],
        {
            gas: 1_000_000n,
        },
    );
    await publicClient.waitForTransactionReceipt({hash});

    console.log(`✅ Minter configured: ${hash}`);

    hash = await usdc.write.mint([account.address, parseUnits("1000000000", 6)], {
        gas: 1_000_000n,
    });
    await publicClient.waitForTransactionReceipt({hash});

    console.log(`✅ Minted to ${account.address}: ${hash}`);

    hash = await usdc.write.approve([rollupProxyAddr, maxUint256], {
        gas: 1_000_000n,
    });
    await publicClient.waitForTransactionReceipt({hash});

    console.log("All mint (test) transactions executed");

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
