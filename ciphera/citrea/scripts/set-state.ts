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

async function main() {
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

    console.log("Resetting RollupV1 state with account:", deployer.address);

    // Get the RollupV1 contract
    const rollupV1Address = process.env.ROLLUP_V1_ADDRESS;
    if (!rollupV1Address) {
        throw new Error("ROLLUP_V1_ADDRESS environment variable is not set");
    }

    const rollupV1 = await ethers.getContractAt("RollupV1", rollupV1Address);

    console.log("\n=== Current State ===");
    const currentBlockHeight = await rollupV1.blockHeight();
    const currentRoot = await rollupV1.currentRootHash();
    console.log("Current Block Height:", currentBlockHeight.toString());
    console.log("Current Root Hash:", currentRoot);

    // Empty merkle tree root hash constant from the contract initialization
    const emptyMerkleRoot =
        "0x0577b5b4aa3eaba75b2a919d5d7c63b7258aa507d38e346bf2ff1d48790379ff";

    console.log("\n=== Resetting State ===");

    // Reset root hash
    console.log("Resetting root hash to empty merkle tree root...");
    let tx = await rollupV1.setRoot(emptyMerkleRoot);
    await tx.wait();
    console.log("✓ Root hash reset");

    // Note: blockHeight cannot be directly reset by owner. If you need to reset it,
    // you would need to modify the contract to add an owner function, or use a proxy upgrade.
    console.log(
        "\nℹ️ Note: blockHeight cannot be directly reset via owner functions."
    );
    console.log(
        "   If you need to reset blockHeight, consider:"
    );
    console.log("   1. Adding an owner function to RollupV1");
    console.log("   2. Using a proxy upgrade pattern");
    console.log("   3. Deploying a fresh instance of RollupV1");

    // Optional: Clear specific mints if you know their hashes
    // const mintHashesToClear = [
    //   "0x...",
    // ];
    // for (const mintHash of mintHashesToClear) {
    //   console.log(`Clearing mint: ${mintHash}`);
    //   // Note: You would need to add a function to the contract to clear mints
    // }

    console.log("\n=== Final State ===");
    const finalBlockHeight = await rollupV1.blockHeight();
    const finalRoot = await rollupV1.currentRootHash();
    console.log("Final Block Height:", finalBlockHeight.toString());
    console.log("Final Root Hash:", finalRoot);

    console.log("\n✓ RollupV1 state reset complete!");
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(error);
        process.exit(1);
    });