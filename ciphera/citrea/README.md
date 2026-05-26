# Citrea Smart Contracts

Rollup smart contracts that verify the rollup state on Citrea. The
`/scripts` subfolder holds the deploy and operational scripts.

Networks:

| Name    | chainId | NETWORK= |
|---------|---------|----------|
| Devnet  | 5655    | dev      |
| Testnet | 5115    | test     |
| Mainnet | 4114    | main     |

Every script targets exactly one of these via the `NETWORK=` env var.
Mainnet RPC, mnemonic, and token addresses must be supplied explicitly —
nothing falls back to a devnet default on a production-shaped script.

Core components:

1. `RollupV1` with transparent proxy upgradeability and an in-contract
   `TimelockController` that owns the rollup after `initialize()`.
2. Aggregate proof verifier deployed separately under `scripts/deploy-verifier.ts`.
3. Operational scripts that route every owner-gated write through the
   timelock (`set-validators`, `set-burn-substitutor`, `set-escrow-manager`,
   `add-token`).
4. Devnet-only tooling under `scripts/devnet/` (USDC fixture deploy,
   local test transactions, etc.) — gated by a chainId check so it
   cannot accidentally run on testnet or mainnet.

## Design notes

Detailed security and operational rationale for the rollup design lives in:

- [`docs/rollup-v1-design-notes.md`](docs/rollup-v1-design-notes.md)
- [`docs/mainnet-prep-plan.md`](docs/mainnet-prep-plan.md) — refactor plan
- [`docs/mainnet-runbook.md`](docs/mainnet-runbook.md) — day-of operator runbook

Keep inline contract comments concise; the design notes are the source
of truth for invariants and tradeoffs.

## What `initialize()` does

The proxy's `initialize` call is the **only** privileged step in a
mainnet deploy. It atomically:

1. Sets the owner, escrow manager, token, verifier, prover, and
   initial validator set.
2. Seeds the empty Merkle root and initial token mapping.
3. Sets the burn fee, fee sink, per-mint cap, global TVL cap, and
   open-proving delay.
4. Deploys a `TimelockController` with the configured min delay,
   proposers, and executors.
5. Transfers rollup ownership to the timelock.

The deploy script then transfers ProxyAdmin ownership to the same
timelock — both ownerships are read back at the end of the script and
the run fails if either is wrong.

Required env vars for `scripts/deploy.ts`:

| Var                          | Required for | Notes                                                |
|------------------------------|--------------|------------------------------------------------------|
| `NETWORK`                    | all          | `dev`, `test`, or `main`                             |
| `MNEMONIC`                   | test, main   | BIP39 seed for the deployer/owner account            |
| `PRIVATE_KEY`                | dev          | hex key for the dev account                          |
| `RPC_URL`                    | optional     | overrides the public RPC for the chosen network      |
| `VERIFIER`                   | all          | pre-deployed aggregate verifier address              |
| `ERC20_ADDRESS`              | all          | wrapped-cBTC (or fixture) token, no default          |
| `INITIAL_NOTE_KIND`          | optional     | 32-byte seed noteKind; derived from chain+token if omitted |
| `BURNER_ADDRESS`             | test, main   | initial escrow manager / burn substitutor            |
| `PER_MINT_CAP_WEI`           | optional     | default `0.001` token                                |
| `GLOBAL_TVL_CAP_WEI`         | optional     | default `10` tokens                                  |
| `OPEN_PROVING_DELAY_SECONDS` | optional     | default `7 days`; must be `>= 7 days`                |
| `BURN_FEE_WEI`               | optional     | default `300 sats`; hard-capped at `3000 sats`       |
| `FEE_SINK`                   | optional     | default deployer                                     |
| `TIMELOCK_MIN_DELAY_SECONDS` | optional     | default `3600` (1h); must be `>= 1h`                 |
| `TIMELOCK_PROPOSERS`         | optional     | csv; default deployer                                |
| `TIMELOCK_EXECUTORS`         | optional     | csv; default deployer. Use `0x0000…0000` for the open-execute sentinel |
| `CONFIRM`                    | main         | must equal chainId (`4114`) on mainnet               |

## Local devnet (Citrea regtest node)

Inside the dev container:

```bash
/citrea --dev --da-layer mock \
  --rollup-config-path /configs/mock/sequencer_rollup_config.toml \
  --sequencer /configs/mock/sequencer_config.toml \
  --genesis-paths /genesis/mock/ \
  > /app/citrea-node.log 2>&1 &
```

Then:

```bash
# 1. Deploy a NoopVerifier (devnet only — accepts any proof).
NETWORK=dev npx hardhat run scripts/devnet/deploy-verifiers-devnet.ts

# 2. Deploy the rollup against that verifier.
NETWORK=dev VERIFIER=0x... ERC20_ADDRESS=0x... \
  npx hardhat run scripts/deploy.ts
```

## Testing

The project uses Hardhat with Mocha/Chai. Run:

```bash
npx hardhat test
```

Key test files in `test/`:

- `RollupV1.test.ts` — core rollup behaviour
- `MainnetDeployParity.test.ts` — deploy script invariants
- `SetValidatorsViaTimelock.test.ts` — owner-gated write via timelock
- `RenounceGuard.test.ts` — renounce safety gate

## Regenerating EVM verifier

To regenerate the aggregate proof verifier binary, see [pkg/prover](/pkg/prover).
After regeneration, the canonical VK hash is written to
`contracts/noir/agg_agg_vk_hash.json` and is read by `deploy.ts` — do
not edit the hash by hand.

## Deployment scripts

Production-shaped scripts (`scripts/`):

| Script                    | Purpose                                                         |
|---------------------------|-----------------------------------------------------------------|
| `deploy.ts`               | Deploy the rollup proxy + initialize + transfer ProxyAdmin      |
| `deploy-verifier.ts`      | Deploy the aggregate Honk verifier (and its transcript library) |
| `deploy-utxo-verifier.ts` | Deploy the UTXO Honk verifier                                   |
| `set-validators.ts`       | Update the validator set for an epoch (timelock-dispatched)     |
| `set-burn-substitutor.ts` | Add/remove a burn substitutor (timelock-dispatched)             |
| `set-escrow-manager.ts`   | Rotate the escrow manager (timelock-dispatched)                 |
| `add-token.ts`            | Register a new noteKind→token mapping (timelock-dispatched)     |
| `send-citrea-tx.ts`       | Smoke-test a transaction against the configured network         |
| `renounce-ownership.ts`   | Permanently renounce rollup + ProxyAdmin ownership (mainnet kill-switch — heavy gating, do not run on day one) |
| `shared.ts`               | Chain configs, timelock helpers, link-bin utility               |

Devnet-only tooling (`scripts/devnet/`):

| Script                       | Purpose                                            |
|------------------------------|----------------------------------------------------|
| `deploy-verifiers-devnet.ts` | Deploy the NoopVerifier on local devnet            |
| `deploy-usdc-devnet.ts`      | Deploy the USDC fixture                            |
| `test-deployment-devnet.ts`  | Initialize USDC v1/v2/v2.1 and mint fixtures       |
| `send-devnet-citrea-tx.ts`   | Smoke transaction against the local devnet RPC     |

Convenience tools (`scripts/utils/`):

| Script              | Purpose                                                |
|---------------------|--------------------------------------------------------|
| `create-wallet.ts`  | Generate a fresh secp256k1 key                         |
| `generate.ts`       | Generate a BIP39 mnemonic + derived account (dev only) |
| `wrap-cbtc.ts`      | Wrap cBTC → WCBTC                                      |
| `unwrap-cbtc.ts`    | Unwrap WCBTC → cBTC                                    |

## Security notes

### Block height monotonicity

`verifyRollup` in `contracts/rollup/RollupV1.sol` requires each new
block height to be strictly greater than the current height. This
prevents:

- **Rollback attacks**: cannot submit blocks with decreasing heights.
- **Replay attacks**: the same block height cannot be reused.
- **Sequencing integrity**: dependent systems can assume monotonic
  height increases.

Validation:

```solidity
require(height > blockHeight, "RollupV1: New block height must be greater than current");
```

### Test coverage

```bash
npx hardhat test test/BlockHeightValidation.test.ts
```
