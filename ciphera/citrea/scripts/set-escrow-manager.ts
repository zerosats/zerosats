import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import { network } from "hardhat";
import {createWalletClient, getContract, http} from "viem";
import {mnemonicToAccount} from "viem/accounts";
import {citreaTestChain} from "./shared";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});

const ROLLUP_ADDRESS = process.env.ROLLUP_ADDRESS as `0x${string}`;
const NEW_ESCROW_MANAGER = process.env.NEW_ESCROW_MANAGER as `0x${string}`;

async function main() {
    if (!ROLLUP_ADDRESS) throw new Error("ROLLUP_ADDRESS env var is not set");
    if (!NEW_ESCROW_MANAGER) throw new Error("NEW_ESCROW_MANAGER env var is not set");
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
    console.log("Rollup address:", ROLLUP_ADDRESS);
    console.log("New escrow manager:", NEW_ESCROW_MANAGER);

    const rollup = getContract({
        address: ROLLUP_ADDRESS,
        abi: rollupV1Artifact.abi,
        client: { public: publicClient, wallet: senderClient },
    });

    const hash = await rollup.write.setEscrowManager(
        [NEW_ESCROW_MANAGER],
        {
            gas: 100_000n,
        },
    );

    console.log(`📝 Transaction hash: ${hash}`);

    const receipt = await publicClient.waitForTransactionReceipt({ hash });

    if (receipt.status !== "success") {
        console.error(`❌ Transaction reverted`, receipt);
        throw new Error("setEscrowManager transaction reverted");
    }
    console.log(`✅ Transaction confirmed in block`);
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Fatal error:", error);
        process.exit(1);
    });
