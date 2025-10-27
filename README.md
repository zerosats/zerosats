# zerosats
A privacy-preserving appchain built on top of Citrea.

## Running Citrea Dev node

In the container launched by

```bash
podman run --network=host -it ciphera:latest
```

it is possible to run citrea devnet node with

```bash
/citrea --dev --da-layer mock --rollup-config-path /configs/mock/sequencer_rollup_config.toml \
--sequencer /configs/mock/sequencer_config.toml --genesis-paths /genesis/mock/ > /app/citrea-node.log 2>&1 &
```

How to check if it is running:

```bash
tail -f citrea-node.log
```

## Running Payy Tests

_With Citrea, tests could finish successfully only in single threaded mode `--test-threads=1`_ 

An example command for running a specific test `rpc::transaction::burn_tx`:

```Rust
cargo test rpc::transaction::burn_tx -- --test-threads=1
```

Optional environment variables allow to set up console print output during deployment.

```Rust
LOG_HARDHAT_DEPLOY_OUTPUT=1 cargo test rpc::elements::list_elements_include_spent -- --test-threads=1
```

RUST_LOG=debug LOG_CITREA_OUTPUT=1 LOG_HARDHAT_DEPLOY_OUTPUT=1 cargo test tests::burn_to -- --test-threads=1