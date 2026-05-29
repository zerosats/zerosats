// Deploy the UTXO Honk verifier (and its ZKTranscriptLib dependency) to the
// configured network. The solc library placeholder is auto-discovered from
// the bin file.

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

  await assertChainId(publicClient, profile.chainId, "deploy-utxo-verifier");

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

  console.log("\n🔍 Deploying utxo ZKTranscriptLib...");
  const utxoTranscriptAddr = await deployBin(
    "noir/utxo_ZKTranscriptLib.bin",
    publicClient,
    walletClient,
  );
  console.log(`✅ utxo ZKTranscriptLib: ${utxoTranscriptAddr}`);

  const verifierBin = (
    await readFile("contracts/noir/utxo_HonkVerifier.bin")
  )
    .toString()
    .trimEnd();
  const placeholders = discoverPlaceholders(verifierBin);
  if (placeholders.length !== 1) {
    throw new Error(
      `Expected exactly one library placeholder in utxo_HonkVerifier.bin, found ${placeholders.length}: ${placeholders.join(", ")}`,
    );
  }
  const [placeholder] = placeholders;
  console.log(`🔗 Linking placeholder ${placeholder} → ${utxoTranscriptAddr}`);

  const utxoVerifierAddr = await deployBin(
    "noir/utxo_HonkVerifier.bin",
    publicClient,
    walletClient,
    { [placeholder]: utxoTranscriptAddr },
  );
  console.log(`✅ UTXO Verifier Contract: ${utxoVerifierAddr}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
