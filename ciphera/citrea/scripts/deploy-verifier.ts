import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import { network } from "hardhat";
import {createWalletClient, formatEther, http} from "viem";
import {mnemonicToAccount} from "viem/accounts";
import {citreaTestChain, deployBin} from "./shared";

// Placeholder embedded by solc for ZKTranscriptLib in agg_agg_HonkVerifier
const AGG_AGG_TRANSCRIPT_PLACEHOLDER = "__$e6391f3e4b1839f34ea5577896c8005de7$__";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});

const ROLLUP_ADDRESS = process.env.ROLLUP_ADDRESS as `0x${string}`;
const NEW_ESCROW_MANAGER = process.env.NEW_ESCROW_MANAGER as `0x${string}`;

async function main() {
    const seed = process.env.MNEMONIC;
    if (!seed) throw new Error("MNEMONIC env var is not set");
    let account = mnemonicToAccount(seed);
    const rpcUrl = process.env.TESTNET_RPC_URL || "https://rpc.testnet.citrea.xyz";

    const publicClient = await viem.getPublicClient();

    let senderClient = createWalletClient({
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

    console.log("Wallet address:", senderClient.account.address);

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

    console.log("\n🔍 Looking for binary files and deploying contracts...");

    const aggAggTranscriptAddr = await deployBin(
        "noir/agg_agg_ZKTranscriptLib.bin",
        publicClient,
        senderClient,
    );
    console.log(`✅ agg_agg ZKTranscriptLib: ${aggAggTranscriptAddr}`);

    const aggregateVerifierAddr = await deployBin(
        "noir/agg_agg_HonkVerifier.bin",
        publicClient,
        senderClient,
        { [AGG_AGG_TRANSCRIPT_PLACEHOLDER]: aggAggTranscriptAddr },
    );

    console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Fatal error:", error);
        process.exit(1);
    });
