# Getting Started with Ciphera CLI Wallet

A step-by-step guide to building and using the Ciphera private payments CLI wallet.

## 📋 Table of Contents

- [Prerequisites](#prerequisites)
- [Building the CLI Wallet](#building-the-cli-wallet)
- [Wallet Operations](#wallet-operations)
  - [Check Contract State](#1-check-contract-state)
  - [Mint Tokens](#2-mint-tokens)
  - [Send Tokens](#3-send-tokens)
  - [Receive Tokens](#4-receive-tokens)
- [Querying the Blockchain](#querying-the-blockchain)
- [Understanding Privacy](#understanding-privacy)
- [Troubleshooting](#troubleshooting)

---

## 🔧 Prerequisites

### Required Software

1. **Rust** (latest stable)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Node.js & npm** (for Solidity compilation)
   ```bash
   # macOS (via Homebrew)
   brew install node
   
   # Or download from https://nodejs.org/
   ```

3. **Git LFS** (for large file support)
   ```bash
   # macOS
   brew install git-lfs
   git lfs install
   ```

4. **Noir & Barretenberg** (ZK proving system)
   ```bash
   # Install Noir
   curl -L https://raw.githubusercontent.com/noir-lang/noirup/refs/heads/main/install | bash
   noirup -v 1.0.0-beta.9
   
   # Install Barretenberg
   curl -L https://raw.githubusercontent.com/AztecProtocol/aztec-packages/refs/heads/master/barretenberg/bbup/install | bash
   bbup
   ```

5. **Additional Dependencies** (macOS)
   ```bash
   brew install openssl pkg-config protobuf
   ```

---

## 🏗️ Building the CLI Wallet

### 1. Clone the Repository

```bash
git clone <repository-url>
cd zerosats/payy
```

### 2. Compile Solidity Contracts

```bash
cd citrea
npm install
npx hardhat compile
cd ..
```

### 3. Build the CLI

```bash
cargo build --release
```

The compiled binary will be at: `target/release/payy-cli`

---

## 💰 Wallet Operations

### Connection Details

For this guide, we'll use our fork of Payy as the payments network and Citrea testnet as the parent chain that hosts the rollup contract and bridge contract:

- **Payy Node**: `63.176.138.198:8091`
- **Citrea Chain ID**: `5115`
- **Citrea wcBTC Token Contract**: `0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93`
- **Citrea Rollup Contract**: `0x1a0E789aa9aE8883C5e55B8E57EFC94F00943d53`
- **Citrea RPC**: `https://rpc.testnet.citrea.xyz`

---

### 1. Check Contract State

Before performing operations, check the current state:

```bash
./target/release/payy-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  --chain 5115 \
  --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  state
```

**Expected Output:**
```
📊 Contract state:
   Root hash: <merkle_root>
   Height: <block_height>
   Safe citrea height: <citrea_block>
```

---

### 2. Mint Tokens

Minting brings tokens from Citrea into the private rollup.

#### Step 2a: Approve ERC20 Spending

First, approve the rollup contract to spend your tokens:

```bash
cast send 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  "approve(address,uint256)" \
  0x1a0E789aa9aE8883C5e55B8E57EFC94F00943d53 \
  100000000000000000000 \
  --rpc-url https://rpc.testnet.citrea.xyz \
  --private-key <YOUR_ETHEREUM_PRIVATE_KEY>
```

> **Note:** You need `cast` from [Foundry](https://book.getfoundry.sh/getting-started/installation)

#### Step 2b: Mint to Your Wallet

```bash
./target/release/payy-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  --chain 5115 \
  --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  mint \
  --secret <YOUR_SECRET_KEY> \
  --height <CURRENT_HEIGHT> \
  --amount 100000000000000
```

**Parameters:**
- `--secret`: Your wallet's private key (32-byte hex)
- `--height`: Current block height from `state` command
- `--amount`: Token amount in wei (e.g., `100000000000000` = 0.0001 tokens with 18 decimals)

**Expected Output:**
```
🔍 Found mint message...
⏳ Generating proof... (this may take 10-30 seconds)
✅ Transaction <hash> has been sent!
   Height: <block>
   Root hash: <new_root>
```

---

### 3. Send Tokens

Transfer tokens privately to another user.

#### Step 3a: Create a Note for the Recipient

```bash
./target/release/payy-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  --chain 5115 \
  --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  spend \
  --out alice-note.json
```

**What happens:**
- Alice's wallet is loaded
- A ZK proof is generated (~5-10 seconds)
- A transaction is submitted to the network
- A note file is created: `alice-note.json`

**Expected Output:**
```
💸 Spending 100000000000000 of 100000000000000
⏳ Generating proof... (this may take 5-10 seconds)
✅ Transaction <hash> has been sent!
   Height: <block>
   Root hash: <new_root>
📝 Note saved to alice-note.json
```

#### Step 3b: Share the Note

**Securely share `alice-note.json` with the recipient** (Bob). This file contains:
- The encrypted note details
- The value
- The secret key to claim it

> ⚠️ **Security:** Anyone with this file can claim the funds! Share it securely.

---

### 4. Receive Tokens

Claim tokens sent to you from a note file.

```bash
./target/release/payy-cli \
  --name bob \
  --host 63.176.138.198 \
  --port 8091 \
  --chain 5115 \
  --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  receive \
  --note alice-note.json
```

**What happens:**
- Bob's wallet loads the note
- A ZK proof is generated to claim the funds
- The transaction is submitted
- Bob now owns the tokens privately

**Expected Output:**
```
🗝 Found note file!
⏳ Generating proof... (this may take 10-30 seconds)
✅ Transaction <hash> has been sent!
   Height: <block>
   Root hash: <new_root>
```

---

## 🔍 Querying the Blockchain

### View a Specific Transaction

```bash
curl -s http://63.176.138.198:8091/v0/transactions/<TRANSACTION_HASH> | jq
```

### View a Specific Block

```bash
curl -s http://63.176.138.198:8091/v0/blocks/<BLOCK_HEIGHT> | jq
```

### List Recent Transactions

```bash
curl -s http://63.176.138.198:8091/v0/transactions | jq
```

### List Recent Blocks

```bash
curl -s http://63.176.138.198:8091/v0/blocks | jq
```

### Check Current Chain Height

```bash
curl -s http://63.176.138.198:8091/v0/height | jq
```

---

## 🎯 Example Workflow

Here's a complete example of Alice sending tokens to Bob:

```bash
# 1. Alice mints 0.0001 tokens
./target/release/payy-cli --name alice --host 63.176.138.198 --port 8091 \
  --chain 5115 --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  mint --secret <secret> --height 95888 --amount 100000000000000

# 2. Alice creates a note for Bob
./target/release/payy-cli --name alice --host 63.176.138.198 --port 8091 \
  --chain 5115 --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  spend --out alice-note.json

# 3. Alice securely shares alice-note.json with Bob

# 4. Bob receives the tokens
./target/release/payy-cli --name bob --host 63.176.138.198 --port 8091 \
  --chain 5115 --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  receive --note alice-note.json

# 5. Query the transaction (amounts remain private!)
curl -s http://63.176.138.198:8091/v0/transactions/<HASH> | jq
```

---


