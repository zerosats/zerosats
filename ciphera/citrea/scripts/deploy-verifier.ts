// Deploy the aggregate Honk verifier (and its ZKTranscriptLib dependency) to
// the configured network. The solc library placeholder is auto-discovered
// from the bin file so VK regenerations that shift the keccak prefix do not
// silently mis-link.

import { readFile } from "fs/promises";
import { createPublicClient, createWalletClient, formatEther, http } from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  deployBin,
  discoverPlaceholders,
  parseNetwork,
  requireEnv,
  resolveRpcUrl,
} from "./shared";

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);

  const account =
    profile.name === "dev"
      ? privateKeyToAccount(requireEnv("PRIVATE_KEY") as `0x${string}`)
      : mnemonicToAccount(requireEnv("MNEMONIC"));

  const publicClient = createPublicClient({
    chain: {
      ...profile.chain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 60_000, retryCount: 3 }),
  });

  await assertChainId(publicClient, profile.chainId, "deploy-verifier");

  const walletClient = createWalletClient({
    account,
    chain: {
      ...profile.chain,
      rpcUrls: {
        default: { http: [rpcUrl] },
        public: { http: [rpcUrl] },
      },
    },
    transport: http(rpcUrl, { timeout: 60_000, retryCount: 3 }),
  });

  console.log("Network:        ", profile.name);
  console.log("Wallet address: ", account.address);
  console.log("Chain ID:       ", profile.chainId);
  console.log("Block:          ", await publicClient.getBlockNumber());
  console.log(
    "Balance:        ",
    formatEther(await publicClient.getBalance({ address: account.address })),
    "cBTC",
  );

  // 1. Deploy the transcript library first.
  console.log("\n🔍 Deploying agg_agg ZKTranscriptLib...");
  const aggAggTranscriptAddr = await deployBin(
    "noir/agg_agg_ZKTranscriptLib.bin",
    publicClient,
    walletClient,
  );
  console.log(`✅ agg_agg ZKTranscriptLib: ${aggAggTranscriptAddr}`);

  // 2. Auto-discover the link placeholder from the verifier bin.
  const verifierBin = (
    await readFile("contracts/noir/agg_agg_HonkVerifier.bin")
  )
    .toString()
    .trimEnd();
  const placeholders = discoverPlaceholders(verifierBin);
  if (placeholders.length !== 1) {
    throw new Error(
      `Expected exactly one library placeholder in agg_agg_HonkVerifier.bin, found ${placeholders.length}: ${placeholders.join(", ")}`,
    );
  }
  const [placeholder] = placeholders;
  console.log(`🔗 Linking placeholder ${placeholder} → ${aggAggTranscriptAddr}`);

  // 3. Deploy the verifier with the library address linked in.
  const aggregateVerifierAddr = await deployBin(
    "noir/agg_agg_HonkVerifier.bin",
    publicClient,
    walletClient,
    { [placeholder]: aggAggTranscriptAddr },
  );
  console.log(`✅ Aggregate Verifier Contract: ${aggregateVerifierAddr}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
