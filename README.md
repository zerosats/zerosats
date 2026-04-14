# Zerosats

A privacy-preserving appchain built on top of Citrea. It is a friendly fork of [Payy](https://github.com/polybase/payy) 
project codebase.

Ciphera services for self-hosting:

1) Citrea node + 2TB storage (\$150-\$200 USD)
2) Testnet4 BTC full node (currently free, should be \$12)
3) Ciphera Validator + Burner (currently free, should be \$12)
4) Ciphera Prover (\$44, but we have to try \$84 instance with 4vCPU)

A plan for mainnet:

- [ ] Bump Nargo & Barretenberg backend version
- [ ] Check up network monitoring tools for nodes (Prometheus, Grafana)
- [ ] Prepare complete network setup deployment scripts
  - [ ] Possibly re-use from Zk-rollup codebase
  - [ ]  Redeploy on AWS according to guidelines & test services
- [ ] Security audits:
- [ ] Noir programs
- [ ] Rollup contract


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

## Running Ciphera Tests

The main entry point for e2e coverage is `./scripts/test.sh`.

- `./scripts/test.sh` runs the non-ignored node e2e tests first, then the ignored full-stack integration tests.
- `./scripts/test.sh --docker` is the most reproducible way to run them if local Citrea/toolchain setup is missing.
- `./scripts/test.sh burn_tx` runs a specific ignored integration test.
- `./scripts/test.sh --verbose` enables Citrea / deploy / node logs.

Note: these are not lightweight tests. Even the non-ignored e2e suite is slow and environment-heavy. In one recent run, the non-ignored phase alone took about 9 minutes 41 seconds, and it depends on Citrea, Hardhat, and Barretenberg/proving setup. So these are harder to run than a normal `cargo test` sanity check.

The runner already executes tests in the required single-threaded mode, so prefer it over ad hoc `cargo test` invocations for the Citrea-backed e2e flows.

## LN Web Control Panel (Own LND Node)

For a button-based local dashboard with persistent settings/history, run:

```bash
node scripts/ln-web-server.mjs
```

Then open:

```text
http://127.0.0.1:8788
```

Data persistence:

- settings + history are stored at `~/.zerosats-ln-web/state.json`
- default bind is `127.0.0.1` (local-only)

Optional env overrides:

- `LN_WEB_HOST` (default `127.0.0.1`)
- `LN_WEB_PORT` (default `8788`)
- `LN_WEB_DATA_DIR` (default `~/.zerosats-ln-web`)

### AWS Network Deployment Notes

#### Citrea Tests

curl -H "Content-Type: application/json" -X POST --data '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":67}' http://172.26.2.88:8080

curl -H "Content-Type: application/json" -X POST --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":67}' http://172.26.2.88:8080

curl -H "Content-Type: application/json" -X POST --data '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":67}' http://172.26.2.88:8080

curl  -H "Content-Type: application/json" -X POST --data '{"jsonrpc":"2.0","method":"eth_getCode","params":["0x6019a8d435da34da0e709a30ba90560c440b7d89"],"id":1}' http://172.26.2.88:8080

### Ciphera Network

sudo docker run --rm -it --entrypoint bash satsbridge/ciphera:latest

./node -c config.toml --mode=validator --rpc-laddr=0.0.0.0:8091 --p2p-laddr=/ip4/0.0.0.0/tcp/5000 --p2p-dial=/ip4/0.0.0.0/tcp/5001
./node -c config.toml --mode=prover --rpc-laddr=0.0.0.0:8092 --p2p-laddr=/ip4/0.0.0.0/tcp/5001 --p2p-dial=/ip4/0.0.0.0/tcp/5000
