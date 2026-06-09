# Ciphera CLI Wallet

A simple CLI wallet for Ciphera private payments network - enabling private, zero-knowledge Bitcoin transactions.

## Features

- 🔒 **Private Payments** - All transactions use zero-knowledge proofs
- 🚀 **Easy to Use** - Simple command-line interface
- 🌐 **Connect to Network** - Connects to Ciphera nodes directly
- 💰 **Mint, Send, Receive** - Full wallet functionality

### Installation

#### Build from Source

**Prerequisites:**
- Rust toolchain from `rust-toolchain.toml`
- Git LFS
- `jq` for parsing node metadata

```bash
git clone https://github.com/zerosats/zerosats.git
cd zerosats/ciphera
git lfs pull
cargo build -p cli --bin ciphera-cli --release
sudo cp target/release/ciphera-cli /usr/local/bin/ciphera-cli
```

### Basic Usage

```bash
# Testnet defaults
export CIPHERA_HOST=https://ciphera.satsbridge.com
export CIPHERA_CHAIN=5115
export CITREA_RPC=https://rpc.testnet.citrea.xyz
export CIPHERA_ROLLUP=$(curl -sS "$CIPHERA_HOST/v0/network" | jq -r '.rollup_contract')

# Create and sync your wallet
ciphera-cli --name alice --host "$CIPHERA_HOST" --chain "$CIPHERA_CHAIN" create
ciphera-cli --name alice --host "$CIPHERA_HOST" --chain "$CIPHERA_CHAIN" sync

# Mint tokens (requires a funded Citrea key with WCBTC and cBTC for gas)
ciphera-cli --name alice \
  --host "$CIPHERA_HOST" \
  --chain "$CIPHERA_CHAIN" \
  --rollup "$CIPHERA_ROLLUP" \
  mint \
  --amount-sat 1000 \
  --secret YOUR_CITREA_PRIVATE_KEY \
  --geth-rpc "$CITREA_RPC"

# Send tokens (create a note for someone)
ciphera-cli --name alice --host "$CIPHERA_HOST" --chain "$CIPHERA_CHAIN" spend --amount-sat 500

# Receive tokens (claim a note someone sent you)
ciphera-cli --name bob --host "$CIPHERA_HOST" --chain "$CIPHERA_CHAIN" receive --note alice-note.json

# Check your balance
cat alice.json
```

## Full Documentation

This README is the current CLI quickstart. The older [Getting Started Guide](../../GettingStarted.md) is not maintained as the source of truth for the CLI.

## Network Details

- **Ciphera Node**: `https://ciphera.satsbridge.com`
- **Citrea Chain ID**: `5115`
- **Citrea wcBTC Token**: `0x4370e27F7d91D9341bFf232d7Ee8bdfE3a9933a0`
- **Rollup Contract**: fetch from `https://ciphera.satsbridge.com/v0/network`
- **Citrea RPC**: `https://rpc.testnet.citrea.xyz`

For mainnet, use `--chain 4114`, a mainnet Ciphera node, `https://rpc.mainnet.citrea.xyz`, and the rollup returned by that node's `/v0/network` response.

## How It Works

1. **Mint**: Bring tokens from Citrea into the private Ciphera network
2. **Send**: Create encrypted notes that can be sent to recipients
3. **Receive**: Claim notes sent to you, adding them to your private balance
4. **ZK Proofs**: All transactions use zero-knowledge proofs to maintain privacy

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.

## Security

⚠️ **This is experimental software. Do not use with real funds.**

For security issues, please contact the team directly.
