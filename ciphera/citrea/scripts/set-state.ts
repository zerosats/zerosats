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
    const version = await rollupV1.read.version();
    console.log("Contract Version:", version.toString());
    console.log("Current Block Height:", currentBlockHeight.toString());
    console.log("Current Root Hash:", currentRoot);

    // Note: as of V2, setRoot is intentionally disabled — it was a
    // direct owner-writable attack surface over the merkle root and
    // its removal is one of the core V2 safety upgrades (Idea 1).
    //
    // This script used to call setRoot to reset the root to the empty
    // merkle tree root. That capability no longer exists on-chain
    // and will not be restored in V2. If you need to reset state on
    // a devnet, redeploy the proxy fresh instead.
    console.log("\n=== Reset Notice ===");
    console.log(
        "setRoot() is disabled in RollupV1 V2 (see Idea 1 in bump-contract)."
    );
    console.log("This script no longer performs any state mutation.");
    console.log("");
    console.log("To reset state on a devnet:");
    console.log("  1. Redeploy the proxy (scripts/deploy.ts) fresh.");
    console.log(
        "  2. Or use the node/test-runner's built-in state reset (hardhat_reset)."
    );
    console.log(
        "  3. Or deploy a brand-new RollupV1 instance side-by-side."
    );
    console.log(
        "\nblockHeight was never resettable via an owner function — that is"
    );
    console.log("unchanged in V2. Redeployment is the only clean reset.");

    console.log("\n✓ Status report complete (no state written).");
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(error);
        process.exit(1);
    });
