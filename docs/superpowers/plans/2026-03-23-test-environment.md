# Ciphera Test Environment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a self-playing, continuously-running, observable local test environment for the Ciphera network — production-shaped, with real ZK proofs and verification, on top of a Citrea regtest with 2-second blocks and unlimited funds.

**Architecture:** A `docker-compose.yml` orchestrates: Citrea regtest (2s blocks), contract deployer (init), Ciphera validator node, Ciphera prover node (real Barretenberg proofs), an auto-player Rust binary, and observability (Jaeger + Prometheus + Grafana). Everything starts with `docker-compose up`.

**Tech Stack:** Rust (auto-player binary), Docker Compose, Jaeger (traces), Prometheus (metrics), Grafana (dashboards), existing Citrea dockerfile, existing Hardhat deploy scripts, existing `cli`/`node`/`contracts`/`prover` crates.

---

## Key Design Decisions

1. **Everything real except Citrea base layer.** Real Honk verifier on-chain (`agg_agg_HonkVerifier.bin`), real ZK proofs from Barretenberg, real rollup verification. Citrea runs `--dev --da-layer mock` with 2-second blocks, pre-funded accounts, no Bitcoin DA. This is a strong dev/test environment, not a production replica — secrets are hardcoded, paths are temp, Grafana is anonymous.

2. **Two Ciphera containers:** One `--mode validator` (block production, consensus, RPC, contract worker) and one `--mode prover` (watches commits via P2P, generates real aggregate proofs, submits rollups). Matches production topology.

3. **No postgres.** Single prover — no multi-prover coordination needed. `prover_database_url` stays `None` (the code handles this at `worker.rs:37-41`).

4. **Auto-player is a workspace crate** reusing `cli` internals (`Wallet`, `NodeClient`, `Utxo.prove()`). Real client-side proofs. Deterministic seed for replayable chaos. The core loop follows note lifecycle: mint → spend → receive → optional burn, then faults on top.

5. **Deploy script fix:** `deploy.ts:40-41` hardcodes `NoopVerifierHonk.bin` for devnet. We make it check `DEV_USE_NOOP_VERIFIER` env var (accepting both `"1"` and `"true"` to match existing test harness at `eth.rs:197`).

6. **Metrics at `/v0/metrics` first pass.** Less invasive than modifying `server.rs`. All routes live under the `/v0` scope (`server.rs:62`). Promoting to top-level `/metrics` can come later if needed.

7. **Slower throughput by design.** Real proofs take seconds. The auto-player delays are 5-30s. This is the point — you observe the real proving pipeline.

---

## File Structure

```
testenv/
├── docker-compose.yml                    # Orchestrates all services
├── config/
│   ├── validator.toml                    # Ciphera validator node config
│   ├── prover.toml                       # Ciphera prover node config
│   ├── prometheus.yml                    # Prometheus scrape config
│   └── grafana/
│       ├── datasources.yml              # Auto-provision Prometheus + Jaeger datasources
│       └── dashboards/
│           ├── dashboards.yml           # Dashboard provisioning config
│           └── ciphera.json             # Pre-baked Grafana dashboard
├── scripts/
│   ├── deploy-and-wait.sh               # Init: wait for Citrea, deploy contracts (real verifier), write addresses
│   ├── entrypoint-validator.sh          # Wait for deployed addresses, start validator
│   └── entrypoint-prover.sh             # Wait for validator, start prover
├── test-smoke.sh                         # Smoke test

ciphera/
├── pkg/
│   └── autoplayer/                       # NEW workspace crate
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                   # Entry point: parse config, seed RNG, spawn player loop
│           ├── player.rs                 # State machine: wallet pool, note lifecycle loop, fault injection
│           ├── actions.rs                # mint, spend, receive, burn, faults — mirrors CLI handlers
│           └── config.rs                 # CLI args: seed, tps, burst, amount range, recipient pool, fail-rate
│   └── node/
│       └── src/
│           ├── node.rs                   # MODIFIED: add pub(crate) getters for mempool_len(), tree_len()
│           └── rpc/routes/
│               ├── metrics.rs            # NEW: Prometheus /v0/metrics endpoint
│               ├── configure.rs          # MODIFIED: register metrics route
│               └── mod.rs                # MODIFIED: add metrics module

ciphera/citrea/scripts/
│   └── deploy.ts                         # MODIFIED: respect DEV_USE_NOOP_VERIFIER env var ("1" or "true")
```

---

## Task 0: Fix deploy.ts to support real verifier in devnet mode

**Files:**
- Modify: `ciphera/citrea/scripts/deploy.ts` (line 40-41)

**Context:** Currently `deploy.ts` hardcodes `NoopVerifierHonk.bin` for all non-testnet deployments. The env vars `DEV_USE_NOOP_VERIFIER` and `DEV_USE_DEPLOYER_AS_POOL_ROLLUP` are set by the test harness (`eth.rs:197,201`) but **never read by deploy.ts** — they're dead code on the JS side. We fix this by making deploy.ts actually consume `DEV_USE_NOOP_VERIFIER`.

- [ ] **Step 1: Modify the verifier selection logic**

In `ciphera/citrea/scripts/deploy.ts`, change line 40-41 from:
```typescript
const maybeNoopVerifier = (verifier: string) =>
    isTestnet ? verifier : "NoopVerifierHonk.bin";
```
To:
```typescript
const useNoopVerifier =
    !isTestnet &&
    (process.env.DEV_USE_NOOP_VERIFIER === "1" || process.env.DEV_USE_NOOP_VERIFIER === "true");
const maybeNoopVerifier = (verifier: string) =>
    useNoopVerifier ? "NoopVerifierHonk.bin" : verifier;
```

This accepts both `"1"` (what `eth.rs:197` sends) and `"true"`. Existing tests set `DEV_USE_NOOP_VERIFIER=1` → still get noop. Our test environment omits it → gets real Honk verifier. Testnet deploys → always get real verifier (unchanged).

- [ ] **Step 2: Verify existing tests are unaffected**

The test harness sets `DEV_USE_NOOP_VERIFIER=1`. With our change, `"1" === "1"` → `useNoopVerifier = true` → noop deployed. Same behavior as before (where devnet always got noop regardless of env var).

Run the fast (non-ignored) e2e tests:
```bash
cd /Users/talip/Desktop/LNX/zerosats/ciphera && cargo test -p node --test e2e -- --test-threads=1
```

- [ ] **Step 3: Commit**

```bash
git add ciphera/citrea/scripts/deploy.ts
git commit -m "feat: deploy.ts reads DEV_USE_NOOP_VERIFIER env var (accepts '1' or 'true')"
```

---

## Task 1: Add Prometheus metrics endpoint to the Ciphera node

**Files:**
- Modify: `ciphera/pkg/node/src/node.rs` (add `pub(crate)` getter methods)
- Create: `ciphera/pkg/node/src/rpc/routes/metrics.rs`
- Modify: `ciphera/pkg/node/src/rpc/routes/configure.rs` (register route)
- Modify: `ciphera/pkg/node/src/rpc/routes/mod.rs` (add module)
- Modify: `ciphera/pkg/node/Cargo.toml` (add `prometheus`)
- Modify: `ciphera/Cargo.toml` (add `prometheus` to workspace deps)
- Modify: `ciphera/pkg/node/src/rpc/routes/txn.rs` (increment counter on tx submission)

**Context:** `NodeShared` fields (`mempool`, `notes_tree`, `block_store`) are private (`node.rs:100,106,112`). The metrics handler is inside the same crate (`node`), so `pub(crate)` getters suffice. Routes live under `/v0` scope (`server.rs:62`), so the endpoint will be at `/v0/metrics`. Prometheus scrapes there.

- [ ] **Step 1: Add workspace + crate dependency**

In `ciphera/Cargo.toml` workspace deps, add:
```toml
prometheus = { version = "0.13", features = ["process"] }
```

In `ciphera/pkg/node/Cargo.toml` deps, add:
```toml
prometheus = { workspace = true }
```

- [ ] **Step 2: Add getter methods to NodeShared**

In `ciphera/pkg/node/src/node.rs`, add these methods to the `impl NodeShared` block (which starts at line 274):

```rust
/// Current mempool size (pending transactions)
pub(crate) fn mempool_len(&self) -> usize {
    self.mempool.len()
}

/// Number of elements in the merkle tree
pub(crate) fn tree_len(&self) -> usize {
    self.notes_tree.read().tree().len()
}
```

The `Mempool` struct may not have a public `len()` method. Check `ciphera/pkg/node/src/mempool.rs`:
- If `Mempool` has no `len()`, add one: `pub fn len(&self) -> usize { self.state.lock().pool.len() }`
- Verify the internal field names — `state` and `pool` are illustrative. Read the actual `Mempool` and `MempoolState` structs.

The existing `height()` is already `pub(crate)` at line 275.

- [ ] **Step 3: Create metrics.rs**

Create `ciphera/pkg/node/src/rpc/routes/metrics.rs`:

```rust
use actix_web::{web, HttpResponse};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use super::state::State;

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

static BLOCK_HEIGHT: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new("ciphera_block_height", "Current block height").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

static MEMPOOL_SIZE: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new("ciphera_mempool_size", "Pending transactions in mempool").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

static TRANSACTIONS_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    let c = IntCounter::new("ciphera_transactions_total", "Total transactions processed").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

static MERKLE_TREE_SIZE: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new("ciphera_merkle_tree_elements", "Elements in merkle tree").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

/// Increment transaction counter. Called from txn.rs on successful submission.
pub fn inc_transactions() {
    TRANSACTIONS_TOTAL.inc();
}

pub async fn get_metrics(state: web::Data<State>) -> HttpResponse {
    let node = &state.node;

    BLOCK_HEIGHT.set(node.height().0 as i64);
    MEMPOOL_SIZE.set(node.mempool_len() as i64);
    MERKLE_TREE_SIZE.set(node.tree_len() as i64);

    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder.encode(&REGISTRY.gather(), &mut buffer).unwrap();
    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();

    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(buffer)
}
```

Uses `std::sync::LazyLock` (stable since Rust 1.80; project uses 1.88). All field access goes through `pub(crate)` getters from Step 2.

- [ ] **Step 4: Register the route inside `/v0` scope**

In `ciphera/pkg/node/src/rpc/routes/configure.rs`, add to the `configure_routes` function:
```rust
.service(web::resource("/metrics").get(metrics::get_metrics))
```

In `ciphera/pkg/node/src/rpc/routes/mod.rs`, add:
```rust
pub mod metrics;
```

Update the import line in `configure.rs`:
```rust
use super::{State, blocks, element, health, height, merkle, metrics, network, smirk, stats, txn};
```

Prometheus config will scrape `ciphera-validator:8091/v0/metrics`.

- [ ] **Step 5: Wire `inc_transactions()` into transaction submission**

In `ciphera/pkg/node/src/rpc/routes/txn.rs`, find `submit_txn`. After the `submit_transaction_and_wait` succeeds, add:
```rust
super::metrics::inc_transactions();
```

- [ ] **Step 6: Verify compilation**

```bash
cd /Users/talip/Desktop/LNX/zerosats/ciphera && cargo check -p node
```

- [ ] **Step 7: Commit**

```bash
git add ciphera/Cargo.toml ciphera/pkg/node/
git commit -m "feat: add Prometheus /v0/metrics endpoint to Ciphera node"
```

---

## Task 2: Create the auto-player crate

**Files:**
- Create: `ciphera/pkg/autoplayer/Cargo.toml`
- Create: `ciphera/pkg/autoplayer/src/main.rs`
- Create: `ciphera/pkg/autoplayer/src/config.rs`
- Create: `ciphera/pkg/autoplayer/src/actions.rs`
- Create: `ciphera/pkg/autoplayer/src/player.rs`

No workspace `Cargo.toml` change needed — `members = ["pkg/*"]` glob covers it.

### Step 1: Scaffold (Cargo.toml + config.rs + stubs)

- [ ] **Step 1a: Create Cargo.toml**

Create `ciphera/pkg/autoplayer/Cargo.toml`:
```toml
[package]
name = "autoplayer"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "autoplayer"
path = "src/main.rs"

[dependencies]
cli = { path = "../cli" }
contracts = { path = "../contracts" }
node-interface = { path = "../node-interface" }
zk-primitives = { path = "../zk-primitives" }
barretenberg = { path = "../barretenberg" }
primitives = { path = "../primitives" }
element = { path = "../element" }
web3 = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
rand = { workspace = true }
rand_chacha = { workspace = true }
clap = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
eyre = { workspace = true }
color-eyre = { workspace = true }
chrono = { workspace = true }
```

- [ ] **Step 1b: Create config.rs with deterministic seed + bounded knobs**

Create `ciphera/pkg/autoplayer/src/config.rs`:
```rust
use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "autoplayer")]
#[command(about = "Ciphera test environment auto-player — generates randomized transaction traffic")]
pub struct Args {
    /// Ciphera node RPC host
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Ciphera node RPC port
    #[arg(long, default_value = "8091")]
    pub port: u16,

    /// Citrea EVM RPC URL (for mint bridge operations)
    #[arg(long, default_value = "http://127.0.0.1:12345")]
    pub evm_rpc_url: String,

    /// Chain ID
    #[arg(long, default_value = "5655")]
    pub chain_id: u64,

    /// Rollup contract address
    #[arg(long)]
    pub rollup_contract: String,

    /// EVM private key for mints (Hardhat account 0 — unlimited funds)
    #[arg(long, default_value = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")]
    pub evm_secret: String,

    /// Number of wallets in the pool
    #[arg(long, default_value = "4")]
    pub wallet_count: usize,

    /// RNG seed for deterministic replay. 0 = random seed.
    #[arg(long, default_value = "0")]
    pub seed: u64,

    /// Minimum delay between actions (ms). Real proofs take seconds.
    #[arg(long, default_value = "5000")]
    pub min_delay_ms: u64,

    /// Maximum delay between actions (ms)
    #[arg(long, default_value = "30000")]
    pub max_delay_ms: u64,

    /// Weight for mint actions (out of 100)
    #[arg(long, default_value = "25")]
    pub weight_mint: u32,

    /// Weight for spend→receive actions (out of 100). Always paired.
    #[arg(long, default_value = "35")]
    pub weight_spend: u32,

    /// Weight for burn actions (out of 100)
    #[arg(long, default_value = "20")]
    pub weight_burn: u32,

    /// Weight for fault injection actions (out of 100)
    #[arg(long, default_value = "10")]
    pub weight_fault: u32,

    /// Weight for self-spend (spend→receive to same wallet, for UTXO churn)
    #[arg(long, default_value = "10")]
    pub weight_self_spend: u32,

    /// Minimum transaction amount
    #[arg(long, default_value = "1000")]
    pub min_amount: u64,

    /// Maximum transaction amount
    #[arg(long, default_value = "100000")]
    pub max_amount: u64,

    /// Directory to store wallet files
    #[arg(long, default_value = "/tmp/autoplayer-wallets")]
    pub wallet_dir: String,
}
```

- [ ] **Step 1c: Create stub main.rs, actions.rs, player.rs**

`main.rs`:
```rust
mod actions;
mod config;
mod player;

use clap::Parser;
use config::Args;
use player::Player;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,autoplayer=debug")),
        )
        .json()
        .init();

    tracing::info!(?args, "autoplayer starting");
    let mut player = Player::new(args)?;
    player.run().await
}
```

`actions.rs` and `player.rs`: create with `todo!()` stubs so it compiles.

- [ ] **Step 1d: Commit scaffold**

```bash
git add ciphera/pkg/autoplayer/
git commit -m "feat: autoplayer crate scaffold with deterministic seed config"
```

### Step 2: actions.rs — note lifecycle operations

- [ ] **Step 2a: Implement actions.rs**

Six functions, each mirroring a CLI handler in `ciphera/pkg/cli/src/main.rs`:

| Function | Mirrors CLI | Lines |
|---|---|---|
| `do_mint` | `handle_mint` | 496-552 |
| `do_spend` | `handle_note_spend` | 271-325 |
| `do_receive` | `handle_receive` | 384-478 |
| `do_burn` | `handle_burn` | 554-623 |
| `do_fault_garbage_proof` | (new) | — |
| `do_fault_double_spend` | (new) | — |

**Critical: implement the full burn flow.** The plan's generic "prepare → prove → submit" pattern does NOT match burn. Burn is a 3-step process (from `handle_burn`):
1. Create burner note via `prepare_receive_note()`, transfer to self via `prepare_spend_to()`, prove, submit
2. Import burner note via `prepare_import_note()`
3. Execute burn via `prepare_burn()` with EVM address, prove, submit

This is 2 separate on-chain transactions plus an off-chain import. Copy the pattern exactly from `handle_burn` at `main.rs:554-623`.

**Spend produces a note. Receive consumes it.** These are always paired in the player loop (Task 2 Step 3). `do_spend` returns the `InputNote` (or `CipheraURL`), then `do_receive` consumes it in the receiver wallet. This closes the loop.

All actions use `#[instrument(skip_all, fields(...))]` for Jaeger traces.

The `admin_mint` bridge call always happens for mints (real proofs, real bridge).

`NodeClient` already has `wallet_dir()` in its builder (`client.rs:94`), and `Wallet::create_in()` / `Wallet::load_from()` exist (`client.rs:108,110`). No new methods needed.

- [ ] **Step 2b: Commit**

```bash
git add ciphera/pkg/autoplayer/src/actions.rs
git commit -m "feat: autoplayer actions (mint, spend, receive, burn, faults)"
```

### Step 3: player.rs — the state machine

- [ ] **Step 3a: Implement player.rs**

**Deterministic RNG:** Use `rand_chacha::ChaCha8Rng` seeded from `--seed`. If seed is 0, use `rand::random()` for a random seed, but log the actual seed so runs are replayable:
```rust
let actual_seed = if args.seed == 0 { rand::random() } else { args.seed };
info!(seed = actual_seed, "RNG initialized — use --seed={actual_seed} to replay");
let rng = ChaCha8Rng::seed_from_u64(actual_seed);
```

**Note lifecycle loop (not action-only dispatch):**

The core loop follows the note lifecycle. Instead of dispatching disconnected actions, it tracks note flow:

```rust
enum Action {
    Mint,           // Create new notes from Citrea bridge
    SpendReceive,   // Spend from A, receive in B (always paired — closes the loop)
    SelfSpend,      // Spend and receive in same wallet (UTXO churn/fragmentation)
    Burn,           // Full 3-step burn back to Citrea
    FaultGarbage,   // Submit corrupted proof
    FaultDoubleSpend, // Replay spent note
}
```

`SpendReceive` is a single action that does both `do_spend` and `do_receive` atomically — the note produced by spend is immediately consumed by receive in a different wallet. No orphaned notes.

`SelfSpend` does spend→receive in the same wallet, which fragments/consolidates UTXOs.

**Wallet pool:** Initialize with `NodeClient::builder().wallet_dir(&args.wallet_dir).build(chain_id, false, true)` — the `create_wallet=true` flag uses `Wallet::create_in()` internally.

**Initial funding:** Mint a large amount into each wallet on startup.

**Fallback:** If spend/burn selected but no wallet has sufficient balance, mint instead.

**Bounded knobs** (all from `Args`):
- `--seed` → deterministic replay
- `--min-delay-ms` / `--max-delay-ms` → TPS control
- `--min-amount` / `--max-amount` → transaction size range
- `--wallet-count` → recipient pool size
- `--weight-*` → action distribution (including `weight_fault` for fail-rate)

- [ ] **Step 3b: Verify compilation**

```bash
cd /Users/talip/Desktop/LNX/zerosats/ciphera && cargo check -p autoplayer
```

- [ ] **Step 3c: Commit**

```bash
git add ciphera/pkg/autoplayer/src/player.rs ciphera/pkg/autoplayer/src/main.rs
git commit -m "feat: autoplayer state machine with note lifecycle loop and deterministic seed"
```

---

## Task 3: Docker Compose and service configs

**Files:**
- Create: `testenv/docker-compose.yml`
- Create: `testenv/config/validator.toml`
- Create: `testenv/config/prover.toml`
- Create: `testenv/config/prometheus.yml`
- Create: `testenv/config/grafana/datasources.yml`
- Create: `testenv/config/grafana/dashboards/dashboards.yml`
- Create: `testenv/config/grafana/dashboards/ciphera.json`
- Create: `testenv/scripts/deploy-and-wait.sh`
- Create: `testenv/scripts/entrypoint-validator.sh`
- Create: `testenv/scripts/entrypoint-prover.sh`

- [ ] **Step 1: Create docker-compose.yml**

```yaml
services:
  # === Citrea regtest (2s blocks, unlimited funds, no real Bitcoin) ===
  citrea:
    build:
      context: ..
      dockerfile: citrea.dockerfile
    ports:
      - "12345:12345"
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:12345", "-X", "POST",
             "-H", "Content-Type: application/json",
             "-d", '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}']
      interval: 2s
      timeout: 5s
      retries: 30

  # === Init: Deploy contracts with REAL Honk verifier ===
  deployer:
    build:
      context: ..
      dockerfile: dev.dockerfile
    depends_on:
      citrea:
        condition: service_healthy
    entrypoint: ["bash", "/app/testenv/scripts/deploy-and-wait.sh"]
    environment:
      - CITREA_RPC=http://citrea:12345
      # DEV_USE_NOOP_VERIFIER is intentionally NOT set → real Honk verifier deployed
    volumes:
      - deploy-output:/deploy-output
      - ./scripts/deploy-and-wait.sh:/app/testenv/scripts/deploy-and-wait.sh:ro

  # === Ciphera VALIDATOR ===
  ciphera-validator:
    build:
      context: ..
      dockerfile: dev.dockerfile
    depends_on:
      deployer:
        condition: service_completed_successfully
    entrypoint: ["bash", "/app/testenv/scripts/entrypoint-validator.sh"]
    ports:
      - "8091:8091"   # RPC
      - "5000:5000"   # P2P
    environment:
      - OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://jaeger:4317
    volumes:
      - deploy-output:/deploy-output:ro
      - ./config/validator.toml:/app/testenv/config/validator.toml:ro
      - ./scripts/entrypoint-validator.sh:/app/testenv/scripts/entrypoint-validator.sh:ro
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:8091/v0/health"]
      interval: 3s
      timeout: 5s
      retries: 60
      start_period: 10s

  # === Ciphera PROVER (real ZK proofs via Barretenberg) ===
  ciphera-prover:
    build:
      context: ..
      dockerfile: dev.dockerfile
    depends_on:
      ciphera-validator:
        condition: service_healthy
    entrypoint: ["bash", "/app/testenv/scripts/entrypoint-prover.sh"]
    environment:
      - OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://jaeger:4317
    volumes:
      - deploy-output:/deploy-output:ro
      - ./config/prover.toml:/app/testenv/config/prover.toml:ro
      - ./scripts/entrypoint-prover.sh:/app/testenv/scripts/entrypoint-prover.sh:ro

  # === Auto-player (real client-side proofs, deterministic seed) ===
  autoplayer:
    build:
      context: ..
      dockerfile: dev.dockerfile
    depends_on:
      ciphera-validator:
        condition: service_healthy
    entrypoint: >
      bash -c "source /deploy-output/env.sh &&
      /app/target/release/autoplayer
        --host ciphera-validator --port 8091
        --evm-rpc-url http://citrea:12345
        --chain-id 5655
        --rollup-contract $$ROLLUP_PROXY
        --seed 42"
    environment:
      - RUST_LOG=info,autoplayer=debug
    volumes:
      - deploy-output:/deploy-output:ro

  # === Jaeger (distributed traces) ===
  jaeger:
    image: jaegertracing/all-in-one:1.57
    ports:
      - "16686:16686"  # UI
      - "4317:4317"    # OTLP gRPC
    environment:
      - COLLECTOR_OTLP_ENABLED=true

  # === Prometheus (metrics — scrapes both validator and prover) ===
  prometheus:
    image: prom/prometheus:v2.51.0
    ports:
      - "9090:9090"
    volumes:
      - ./config/prometheus.yml:/etc/prometheus/prometheus.yml:ro

  # === Grafana (dashboards) ===
  grafana:
    image: grafana/grafana:10.4.0
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
    volumes:
      - ./config/grafana/datasources.yml:/etc/grafana/provisioning/datasources/datasources.yml:ro
      - ./config/grafana/dashboards/dashboards.yml:/etc/grafana/provisioning/dashboards/dashboards.yml:ro
      - ./config/grafana/dashboards/ciphera.json:/var/lib/grafana/dashboards/ciphera.json:ro

volumes:
  deploy-output:
```

- [ ] **Step 2: Create validator.toml**

```toml
env-name = "testenv"
block-txns-count = 6
min-block-duration = 2000
mode = "validator"
secret-key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
rpc-laddr = "0.0.0.0:8091"
db-path = "/tmp/ciphera-validator/db"
smirk-path = "/tmp/ciphera-validator/smirk"
evm-rpc-url = "http://citrea:12345"
chain-id = 5655
rollup-contract-addr = "PLACEHOLDER"
health-check-commit-interval-sec = 300
rollup-wait-time-ms = 10000
safe-eth-height-offset = 1

[p2p]
laddr = "/ip4/0.0.0.0/tcp/5000"
dial = ""
idle-timeout-secs = 0
whitelisted-ips = []
```

- [ ] **Step 3: Create prover.toml**

```toml
env-name = "testenv"
block-txns-count = 6
min-block-duration = 2000
mode = "prover"
secret-key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
rpc-laddr = "0.0.0.0:8092"
db-path = "/tmp/ciphera-prover/db"
smirk-path = "/tmp/ciphera-prover/smirk"
evm-rpc-url = "http://citrea:12345"
chain-id = 5655
rollup-contract-addr = "PLACEHOLDER"
health-check-commit-interval-sec = 300
rollup-wait-time-ms = 10000
safe-eth-height-offset = 1

[p2p]
laddr = "/ip4/0.0.0.0/tcp/5001"
dial = "/ip4/ciphera-validator/tcp/5000"
idle-timeout-secs = 0
whitelisted-ips = []
```

> **Key differences:** `mode = "prover"`, separate db/smirk paths, `p2p.dial` points to the validator, separate RPC port 8092 (not exposed to host).

- [ ] **Step 4: Create deploy-and-wait.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail

CITREA_RPC="${CITREA_RPC:-http://citrea:12345}"

echo "=== Contract deployer (REAL Honk verifier) ==="
echo "Waiting for Citrea at ${CITREA_RPC}..."

for i in $(seq 1 60); do
    if curl -sf "$CITREA_RPC" -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        > /dev/null 2>&1; then
        echo "Citrea is up (attempt $i)."
        break
    fi
    sleep 1
done

sleep 3

echo "Deploying contracts..."
cd /app/citrea

export TESTING_URL="$CITREA_RPC"
export SECRET_KEY="ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# DEV_USE_NOOP_VERIFIER intentionally NOT exported → real Honk verifier

OUTPUT=$(npx hardhat run scripts/deploy.ts --network citreaDevnet 2>&1)
echo "$OUTPUT"

DEPLOY_JSON=$(echo "$OUTPUT" | grep "DEPLOY_OUTPUT=" | sed 's/DEPLOY_OUTPUT=//')
ROLLUP_PROXY=$(echo "$DEPLOY_JSON" | jq -r '.rollupProxy')
ERC20_ADDR=$(echo "$DEPLOY_JSON" | jq -r '.erc20')
VERIFIER_ADDR=$(echo "$DEPLOY_JSON" | jq -r '.verifier')

echo "Rollup proxy:  $ROLLUP_PROXY"
echo "ERC20:         $ERC20_ADDR"
echo "Verifier:      $VERIFIER_ADDR (real Honk)"

mkdir -p /deploy-output
cat > /deploy-output/env.sh << EOF
export ROLLUP_PROXY="$ROLLUP_PROXY"
export ERC20_ADDR="$ERC20_ADDR"
export VERIFIER_ADDR="$VERIFIER_ADDR"
export CITREA_RPC="$CITREA_RPC"
EOF

echo "$DEPLOY_JSON" > /deploy-output/addresses.json
echo "=== Deployment complete ==="
```

> **Note to implementer:** If `--network citreaDevnet` fails, the deploy script at line 73 already falls back to `TESTING_URL` env var. But Hardhat might still need the network flag for chain config. Check `hardhat.config.ts` for the `citreaDevnet` network definition.

- [ ] **Step 5: Create entrypoint-validator.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== Ciphera VALIDATOR startup ==="
source /deploy-output/env.sh
echo "Using rollup contract: $ROLLUP_PROXY"

sed "s|PLACEHOLDER|${ROLLUP_PROXY}|g" /app/testenv/config/validator.toml > /tmp/validator.toml

cd /app
if [[ ! -f target/release/node ]]; then
    echo "Building Ciphera node..."
    cargo build --release -p node
fi

echo "Starting validator..."
exec ./target/release/node \
    -c /tmp/validator.toml \
    --mode validator \
    --evm-rpc-url "http://citrea:12345" \
    --rollup-contract-addr "$ROLLUP_PROXY"
```

- [ ] **Step 6: Create entrypoint-prover.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== Ciphera PROVER startup ==="
source /deploy-output/env.sh
echo "Using rollup contract: $ROLLUP_PROXY"

sed "s|PLACEHOLDER|${ROLLUP_PROXY}|g" /app/testenv/config/prover.toml > /tmp/prover.toml

cd /app
if [[ ! -f target/release/node ]]; then
    echo "Building Ciphera node..."
    cargo build --release -p node
fi

echo "Starting prover (real ZK proofs via Barretenberg)..."
exec ./target/release/node \
    -c /tmp/prover.toml \
    --mode prover \
    --evm-rpc-url "http://citrea:12345" \
    --rollup-contract-addr "$ROLLUP_PROXY"
```

- [ ] **Step 7: Create prometheus.yml (scrapes BOTH validator and prover)**

```yaml
global:
  scrape_interval: 5s
  evaluation_interval: 5s

scrape_configs:
  - job_name: "ciphera-validator"
    static_configs:
      - targets: ["ciphera-validator:8091"]
    metrics_path: "/v0/metrics"

  - job_name: "ciphera-prover"
    static_configs:
      - targets: ["ciphera-prover:8092"]
    metrics_path: "/v0/metrics"
```

> Both services expose `/v0/metrics`. The prover also runs an RPC server (at port 8092), which includes the metrics endpoint. Grafana can distinguish via `job` label.

- [ ] **Step 8: Create Grafana datasources + dashboard provisioning + dashboard JSON**

`testenv/config/grafana/datasources.yml`:
```yaml
apiVersion: 1
datasources:
  - name: Prometheus
    type: prometheus
    access: proxy
    url: http://prometheus:9090
    isDefault: true
    editable: true
  - name: Jaeger
    type: jaeger
    access: proxy
    url: http://jaeger:16686
    editable: true
```

`testenv/config/grafana/dashboards/dashboards.yml`:
```yaml
apiVersion: 1
providers:
  - name: "default"
    orgId: 1
    folder: ""
    type: file
    disableDeletion: false
    editable: true
    options:
      path: /var/lib/grafana/dashboards
      foldersFromFilesStructure: false
```

`testenv/config/grafana/dashboards/ciphera.json`: Create a dashboard with panels for:
- Block height (stat + time series) — from validator
- Mempool size (stat + time series) — from validator
- Transaction rate (time series, `rate(ciphera_transactions_total[30s])`) — from validator
- Merkle tree growth (time series) — from validator
- Prover block height (stat) — from prover job, shows how far behind prover is vs validator
- Prover merkle tree (stat) — from prover job

All panels should use `{job="ciphera-validator"}` or `{job="ciphera-prover"}` label selectors.

> **Note to implementer:** The dashboard JSON from the previous plan version works as a starting point. Add the prover panels and update `metrics_path` references to `/v0/metrics`.

- [ ] **Step 9: Commit**

```bash
git add testenv/
git commit -m "feat: docker-compose test environment with real prover and pipeline observability"
```

---

## Task 4: Update dev.dockerfile to build binaries

**Files:**
- Modify: `dev.dockerfile`

- [ ] **Step 1: Add cargo build after hardhat compile**

After the existing `npx hardhat compile` line, add:
```dockerfile
WORKDIR /app
RUN cargo build --release -p node -p autoplayer
```

- [ ] **Step 2: Verify docker build**

```bash
cd /Users/talip/Desktop/LNX/zerosats && docker build -f dev.dockerfile -t satsbridge/ciphera:dev .
```

- [ ] **Step 3: Commit**

```bash
git add dev.dockerfile
git commit -m "feat: build node and autoplayer binaries in dev docker image"
```

---

## Task 5: Smoke test

**Files:**
- Create: `testenv/test-smoke.sh`

- [ ] **Step 1: Create smoke test script**

A bash script that:
1. `docker compose build`
2. `docker compose up -d`
3. Wait for validator health (up to 120s)
4. Check block height is advancing
5. Check Prometheus has 2 active scrape targets (validator + prover)
6. Check Grafana health
7. Check Jaeger health
8. Check autoplayer logs for "round start"
9. Check prover logs for "Proving commit" or "Finished proving commit"
10. Check `/v0/metrics` returns `ciphera_` prefixed metrics
11. Print service URLs and leave stack running

- [ ] **Step 2: Run it**

```bash
chmod +x testenv/test-smoke.sh && ./testenv/test-smoke.sh
```

- [ ] **Step 3: Commit**

```bash
git add testenv/test-smoke.sh
git commit -m "feat: smoke test for test environment"
```

---

## Task 6: Compilation fixes and API adaptation

**Context:** Plan code is based on deep analysis but some details will be off. Dedicated debugging pass.

- [ ] **Step 1: Verify `cli` crate public exports**

Read `ciphera/pkg/cli/src/lib.rs`. The autoplayer needs: `Wallet`, `NodeClient`, `NodeClientBuilder`, `note_url::CipheraURL`, `note_url::decode_url`, `Prove` trait. Make anything private `pub` as needed.

- [ ] **Step 2: Verify NodeShared getters compile**

The `pub(crate) fn mempool_len()` and `pub(crate) fn tree_len()` from Task 1 Step 2 depend on:
- `Mempool::len()` existing (add if not)
- `notes_tree.read().tree().len()` working (verify smirk tree API)

- [ ] **Step 3: Verify full compilation**

```bash
cd /Users/talip/Desktop/LNX/zerosats/ciphera && cargo check -p node -p autoplayer
```

- [ ] **Step 4: Commit fixes**

```bash
git add -A
git commit -m "fix: adapt autoplayer and metrics to actual crate APIs"
```

---

## Summary

```
docker compose -f testenv/docker-compose.yml up
```

| Service | URL | What |
|---|---|---|
| Citrea regtest | `localhost:12345` | 2s blocks, unlimited funds, mock DA |
| Ciphera validator | `localhost:8091` | Block production, RPC, consensus |
| Ciphera prover | (internal `:8092`) | Real Barretenberg proofs, rollup submission |
| Autoplayer | (internal) | Note lifecycle: mint→spend→receive→burn + faults |
| Grafana | `localhost:3000` | Dashboards (admin/admin) |
| Jaeger | `localhost:16686` | Distributed traces |
| Prometheus | `localhost:9090` | Metrics (scrapes validator + prover) |

**Production-shaped, not production-identical.** Real ZK proofs, real Honk verifier, real rollup verification, real note lifecycle. Dev secrets, temp paths, anonymous Grafana.

Replay any run: `--seed 42`. Tune traffic: `--weight-mint 50 --weight-spend 30 --min-delay-ms 10000`.

To play manually alongside:
```bash
./target/release/ciphera-cli --name manual --host localhost --port 8091 --no-tls --chain 5655 create
./target/release/ciphera-cli --name manual --host localhost --port 8091 --no-tls --chain 5655 sync
```
