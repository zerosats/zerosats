import rollupV1Artifact from "../artifacts/contracts/rollup2/RollupV1.sol/RollupV1.json";
import { network, viem } from "hardhat";
import { readFile } from "fs/promises";

async function main() {
  // Connect to the "citrea" network (make sure it's defined in hardhat.config.ts)
  const { viem } = await network.connect({
    network: "citreaTestnet",
    chainId: 5115,
  });

  const publicClient = await viem.getPublicClient();
  const [owner] = await viem.getWalletClients();

  const rollupV1 = await owner.deployContract({
    abi: rollupV1Artifact.abi,
    bytecode: rollupV1Artifact.bytecode,
  });

  console.log(`📝 Transaction hash: ${rollupV1}`);

  const receipt = await publicClient.waitForTransactionReceipt({
    hash: rollupV1,
  });

  if (receipt.status == "success") {
    console.log(`✅ Transaction confirmed in block`);
  } else {
    console.log(`❌ Transaction reverted`);
  }

  const rollupV1Addr = receipt.contractAddress;

  console.log(`✅ Transaction confirmed in block`);

  console.log(`✅ Rollup Contract (Implementation): ${rollupV1Addr}`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
