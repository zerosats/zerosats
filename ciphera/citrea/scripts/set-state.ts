import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";

import {
    createPublicClient,
    createWalletClient,
    http,
    getContract,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { citreaDevChain } from "./shared";

async function main() {
    const rollupV1Address = process.env.ROLLUP_V1_ADDRESS as `0x${string}`;
    if (!rollupV1Address) {
        throw new Error("ROLLUP_V1_ADDRESS environment variable is not set");
    }

    const rpcUrl = process.env.TESTING_URL || "http://localhost:12345";
    const privateKey = (process.env.PRIVATE_KEY ||
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80") as `0x${string}`;
    const account = privateKeyToAccount(privateKey);

    const chain = {
        ...citreaDevChain,
        rpcUrls: {
            default: { http: [rpcUrl] },
            public: { http: [rpcUrl] },
        },
    };

    const publicClient = createPublicClient({
        chain,
        transport: http(rpcUrl, { timeout: 30000, retryCount: 3 }),
    });

    const walletClient = createWalletClient({
        account,
        chain,
        transport: http(rpcUrl, { timeout: 30000, retryCount: 3 }),
    });

    console.log("Resetting RollupV1 state with account:", account.address);

    const rollupV1 = getContract({
        address: rollupV1Address,
        abi: rollupV1Artifact.abi,
        client: { public: publicClient, wallet: walletClient },
    });

    console.log("\n=== Current State ===");
    const currentBlockHeight = await rollupV1.read.blockHeight();
    const currentRoot = await rollupV1.read.currentRootHash();
    console.log("Current Block Height:", currentBlockHeight.toString());
    console.log("Current Root Hash:", currentRoot);

    // Empty merkle tree root hash constant from the contract initialization
    const emptyMerkleRoot =
        "0x0577b5b4aa3eaba75b2a919d5d7c63b7258aa507d38e346bf2ff1d48790379ff";

    console.log("\n=== Resetting State ===");
    console.log("Resetting root hash to empty merkle tree root...");

    const hash = await rollupV1.write.setRoot([emptyMerkleRoot]);
    const receipt = await publicClient.waitForTransactionReceipt({ hash });

    if (receipt.status !== "success") {
        throw new Error("setRoot transaction reverted");
    }
    console.log("✓ Root hash reset");

    // Note: blockHeight cannot be directly reset by owner. If you need to reset it,
    // you would need to modify the contract to add an owner function, or use a proxy upgrade.
    console.log(
        "\nNote: blockHeight cannot be directly reset via owner functions."
    );
    console.log("   If you need to reset blockHeight, consider:");
    console.log("   1. Adding an owner function to RollupV1");
    console.log("   2. Using a proxy upgrade pattern");
    console.log("   3. Deploying a fresh instance of RollupV1");

    console.log("\n=== Final State ===");
    const finalBlockHeight = await rollupV1.read.blockHeight();
    const finalRoot = await rollupV1.read.currentRootHash();
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
