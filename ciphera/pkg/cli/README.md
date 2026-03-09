# Ciphera CLI Wallet

A simple CLI wallet for Ciphera private payments network - enabling private, zero-knowledge Bitcoin transactions.

## Features

- 🔒 **Private Payments** - All transactions use zero-knowledge proofs
- 🚀 **Easy to Use** - Simple command-line interface
- 🌐 **Connect to Network** - Connects to Ciphera nodes directly
- 💰 **Mint, Send, Receive** - Full wallet functionality

## Quick Start

Look into [Getting Started](../../GettingStarted.md) document.

### Installation

#### Option 1: Download Pre-compiled Binary (Easiest)

Download the latest release for your platform from [Releases](https://github.com/zerosats/ciphera-cli/releases):

**macOS (Apple Silicon)**
```bash
curl -L https://github.com/zerosats/ciphera-cli/releases/latest/download/ciphera-cli-macos-arm64 -o ciphera-cli
chmod +x ciphera-cli
sudo mv ciphera-cli /usr/local/bin/
```

**macOS (Intel)**
```bash
curl -L https://github.com/zerosats/ciphera-cli/releases/latest/download/ciphera-cli-macos-amd64 -o ciphera-cli
chmod +x ciphera-cli
sudo mv ciphera-cli /usr/local/bin/
```

**Linux**
```bash
curl -L https://github.com/zerosats/ciphera-cli/releases/latest/download/ciphera-cli-linux-amd64 -o ciphera-cli
chmod +x ciphera-cli
sudo mv ciphera-cli /usr/local/bin/
```

#### Option 2: Install from Source (For Developers)

**Prerequisites:**
- Rust 1.70+
- Git LFS

```bash
git clone https://github.com/zerosats/ciphera-cli
cd ciphera-cli
cargo build --release
sudo cp target/release/ciphera-cli /usr/local/bin/ciphera-cli
```

#### Option 3: Using Cargo

```bash
cargo install --git https://github.com/zerosats/ciphera-cli
```

### Basic Usage

```bash
# Connect to the Ciphera network and create your wallet
ciphera-cli --name alice connect

# Mint tokens (bring tokens into the private network)
ciphera-cli --name alice mint \
  --amount 100000000000000 \
  --secret YOUR_PRIVATE_KEY \
  --geth-rpc https://rpc.testnet.citrea.xyz

# Send tokens (create a note for someone)
ciphera-cli --name alice spend --amount 100000000000000

# Receive tokens (claim a note someone sent you)
ciphera-cli --name bob receive --note alice-note.json

# Check your balance
cat alice.json
```

## Full Documentation

See [Getting Started Guide](../../GettingStarted.md) for detailed instructions.

## Network Details

- **Ciphera Node**: `ciphera.satsbridge.com:8091`
- **Citrea Chain ID**: `5115`
- **Citrea wcBTC Token**: `0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93`
- **Rollup Contract**: `0x26c698fa720806f93d94432d430415e3d15d3539`
- **Citrea RPC**: `https://rpc.testnet.citrea.xyz`

## How It Works

1. **Mint**: Bring tokens from Citrea testnet into the private Ciphera network
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

