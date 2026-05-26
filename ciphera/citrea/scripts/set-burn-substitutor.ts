// Add or remove a burn-substitutor allowlist entry on the rollup. Always
// dispatched via the rollup's TimelockController.

import rollupV1Artifact from "../artifacts/contracts/rollup/RollupV1.sol/RollupV1.json";
import {
  createPublicClient,
  createWalletClient,
  encodeFunctionData,
  getContract,
  http,
} from "viem";
import { mnemonicToAccount, privateKeyToAccount } from "viem/accounts";
import {
  assertChainId,
  parseNetwork,
  parseTimelockMode,
  requireEnv,
  resolveRpcUrl,
  timelockDispatch,
} from "./shared";

async function main() {
  const profile = parseNetwork(process.env.NETWORK);
  const rpcUrl = resolveRpcUrl(profile);
  const rollupAddress = requireEnv("ROLLUP_ADDRESS") as `0x${string}`;
  const burnSubstitutor = requireEnv("BURN_SUBSTITUTOR") as `0x${string}`;
  const action = requireEnv("ACTION").toLowerCase();
  if (action !== "add" && action !== "remove") {
    throw new Error("ACTION env var must be either 'add' or 'remove'");
  }
  const mode = parseTimelockMode(process.env.MODE);
  const salt = process.env.TIMELOCK_SALT as `0x${string}` | undefined;

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

  await assertChainId(publicClient, profile.chainId, "set-burn-substitutor");

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

  console.log("Network:         ", profile.name);
  console.log("Wallet address:  ", account.address);
  console.log("Rollup address:  ", rollupAddress);
  console.log("Action:          ", action);
  console.log("Burn substitutor:", burnSubstitutor);
  console.log("Mode:            ", mode);

  const rollup = getContract({
    address: rollupAddress,
    abi: rollupV1Artifact.abi,
    client: { public: publicClient, wallet: walletClient },
  });

  const timelock = (await rollup.read.timelock()) as `0x${string}`;

  const functionName =
    action === "add" ? "addBurnSubstitutor" : "removeBurnSubstitutor";
  const data = encodeFunctionData({
    abi: rollupV1Artifact.abi,
    functionName,
    args: [burnSubstitutor],
  });

  await timelockDispatch({
    publicClient,
    walletClient,
    timelock,
    target: rollupAddress,
    data,
    mode,
    salt,
  });
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
