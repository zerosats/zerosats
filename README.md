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

fn run(&mut self, log_output: Option<bool>) {
let mut command = Command::new(find_binary());

        command
            .arg("--db-path")
            .arg(self.root_dir.path().join("db"));
        command
            .arg("--smirk-path")
            .arg(self.root_dir.path().join("smirk"));
        command
            .arg("--rpc-laddr")
            .arg(format!("127.0.0.1:{}", self.api_port));
        command
            .arg("--p2p-laddr")
            .arg(format!("/ip4/127.0.0.1/tcp/{}", self.p2p_port));
        command
            .arg("--secret-key")
            .arg(format!("0x{}", hex::encode(self.secret_key)));
        command
            .arg("--rollup-contract-addr")
            .arg(format!("0x{:x}", self.rollup_contract_addr));
        command.arg("--evm-rpc-url").arg(self.eth_node.rpc_url());

        command.arg("--p2p-dial").arg(
            self.peers
                .iter()
                .map(|p| format!("/ip4/127.0.0.1/tcp/{}", p.p2p_port))
                .collect::<Vec<_>>()
                .join(","),
        );

        command.arg("--mode").arg(if self.prover {
            "mock-prover"
        } else {
            "validator"
        });

        command.env(
            "POLY_SAFE_ETH_HEIGHT_OFFSET",
            self.safe_eth_height_offset.to_string(),
        );

        let should_log = log_output.unwrap_or(
            std::env::var("LOG_NODE_OUTPUT")
                .map(|v| v == "1")
                .unwrap_or(false),
        );
        let output_piped = if !should_log {
            command
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            true
        } else {
            false
        };

        let mut process = command.spawn().expect("Failed to start node");

        let stdout_sender = self.stdout_sender.take().unwrap();
        let stderr_sender = self.stderr_sender.take().unwrap();

        if output_piped {
            let mut stdout = process.stdout.take().unwrap();
            let mut stderr = process.stderr.take().unwrap();

            self.output_readers.push(std::thread::spawn(move || {
                let mut text = Vec::<u8>::new();
                stdout.read_to_end(&mut text).unwrap();

                let text = String::from_utf8_lossy(&text);
                let _ = stdout_sender.send(text.to_string());
            }));

            self.output_readers.push(std::thread::spawn(move || {
                let mut text = Vec::<u8>::new();
                stderr.read_to_end(&mut text).unwrap();

                let text = String::from_utf8_lossy(&text);
                let _ = stderr_sender.send(text.to_string());
            }));
        }

        println!(
            "Node started: {}; Base URL: {}",
            process.id(),
            self.base_url()
        );

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
