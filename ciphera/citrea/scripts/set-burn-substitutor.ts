import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import { network } from "hardhat";
import { createWalletClient, getContract, http } from "viem";
import { mnemonicToAccount } from "viem/accounts";
import { citreaTestChain } from "./shared";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});

const ROLLUP_ADDRESS = process.env.ROLLUP_ADDRESS as `0x${string}`;
const BURN_SUBSTITUTOR = process.env.BURN_SUBSTITUTOR as `0x${string}`;
const ACTION = (process.env.ACTION || "").toLowerCase(); // "add" | "remove"

async function main() {
    if (!ROLLUP_ADDRESS) throw new Error("ROLLUP_ADDRESS env var is not set");
    if (!BURN_SUBSTITUTOR) throw new Error("BURN_SUBSTITUTOR env var is not set");
    if (ACTION !== "add" && ACTION !== "remove") {
        throw new Error("ACTION env var must be either 'add' or 'remove'");
    }

    const seed = process.env.MNEMONIC;
    if (!seed) throw new Error("MNEMONIC env var is not set");
    const account = mnemonicToAccount(seed);
    const rpcUrl = process.env.TESTNET_RPC_URL || "https://rpc.testnet.citrea.xyz";

    const publicClient = await viem.getPublicClient();

    const senderClient = createWalletClient({
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
    console.log("Rollup address:", ROLLUP_ADDRESS);
    console.log("Action:", ACTION);
    console.log("Burn substitutor:", BURN_SUBSTITUTOR);

    const rollup = getContract({
        address: ROLLUP_ADDRESS,
        abi: rollupV1Artifact.abi,
        client: { public: publicClient, wallet: senderClient },
    });

    const hash =
        ACTION === "add"
            ? await rollup.write.addBurnSubstitutor([BURN_SUBSTITUTOR], {
                gas: 100_000n,
            })
            : await rollup.write.removeBurnSubstitutor([BURN_SUBSTITUTOR], {
                gas: 100_000n,
            });

    console.log(`📝 Transaction hash: ${hash}`);

    const receipt = await publicClient.waitForTransactionReceipt({ hash });

    if (receipt.status !== "success") {
        console.error(`❌ Transaction reverted`, receipt);
        throw new Error(`${ACTION}BurnSubstitutor transaction reverted`);
    }
    console.log(`✅ Transaction confirmed in block ${receipt.blockNumber}`);
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Fatal error:", error);
        process.exit(1);
    });