import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import proxyArtifact
    from "../openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol/TransparentUpgradeableProxy.json";
import { network } from "hardhat";
import {parseEther, encodeFunctionData, decodeFunctionResult, getContract, parseUnits} from "viem";

const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
});

const ROLLUP_ADDRESS = "0xcac0d0901ac8806160acc8ef373117898a51dfe7";

async function main() {
    const publicClient = await viem.getPublicClient();
    const [senderClient] = await viem.getWalletClients();

    console.log("Wallet address:", senderClient.account.address);

    const NEW_KIND = "0x000200000000000013fb52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b60000";
    const NEW_TOKEN = "0x52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6";

    const rollup = getContract({
        address: ROLLUP_ADDRESS,
        abi: rollupV1Artifact.abi,
        client: {public: publicClient, wallet: senderClient},
    });

    let hash = await rollup.write.addToken(
        [NEW_KIND, NEW_TOKEN],
        {
            gas: 1_000_000n,
        },
    );

    const receipt = await publicClient.waitForTransactionReceipt({ hash });

    if (receipt.status == "success") {
        console.log(`✅ Transaction confirmed in block`);
    } else {
        console.log(`❌ Transaction reverted`);
    }
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Fatal error:", error);
        process.exit(1);
    });
