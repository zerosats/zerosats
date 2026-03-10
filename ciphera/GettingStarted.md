# Getting Started with Ciphera CLI Wallet

A step-by-step guide to building and using the Ciphera private payments CLI wallet.

## Prerequisites

### Required Software

### 1.  **Rust** (latest stable)
   
   ```bash
   # Check if Rust is installed
   rustc --version && cargo --version
   ```
   
   ```bash
   # If not installed, or to update to latest:
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
   
   #### If already installed, update with:
   ```bash
   rustup update
   ```

### 2. **Node.js & npm** (for Solidity compilation)
   ```bash
   # macOS (via Homebrew)
   brew install node
   
   # Or download from https://nodejs.org/
   ```

### 3. **Git LFS** (for large file support)
   ```bash
   # macOS
   brew install git-lfs
   git lfs install
   ```

### 4. **Noir & Barretenberg** (ZK proving system)
   ```bash
   # Install Noir
   curl -L https://raw.githubusercontent.com/noir-lang/noirup/refs/heads/main/install | bash
   noirup -v 1.0.0-beta.9
   ```
   
   ```bash
   # Install Barretenberg
   curl -L https://raw.githubusercontent.com/AztecProtocol/aztec-packages/refs/heads/master/barretenberg/bbup/install | bash
   bbup -v 1.0.0-nightly.20250723
   ```

### 5. **Additional Dependencies** (macOS)
   ```bash
   brew install openssl pkg-config protobuf
   ```

---

## Building the CLI Wallet

### 1. Clone the Repository (wCBTC Branch)

```bash
git clone <https://github.com/zerosats/zerosats/tree/wcbtc-janusz-fork>
cd zerosats/ciphera
```

### 2. Compile Solidity Contracts

```bash
cd ciphera/citrea
npm install
npx hardhat compile
cd ..
```

### 3. Build the CLI (make sure you're in the project directory)

```bash
cargo build --release
```

The compiled binary will be at: `target/release/ciphera-cli`

---

## Wallet Operations

### Connection Details

For this guide, we'll use Ciphera as the payments network and Citrea testnet as the parent chain that hosts the rollup contract and bridge contract. Relevant information:

- **Ciphera Node**: `ciphera.satsbridge.com`
- **Citrea Chain ID**: `5115`
- **Citrea wcBTC Token Contract**: `0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93`
- **Our Rollup Contract on Citrea**: `0x26c698fa720806f93d94432d430415e3d15d3539`
- **Citrea RPC**: `https://rpc.testnet.citrea.xyz`

---

### 1. Connect to Ciphera Network

We are running a node on AWS. You'll need to connect to it before performing operations. This command also creates your wallet:

```bash
./target/release/ciphera-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  connect
```

**Expected Output:**
```
✅ Node Health Check Passed!
   Current Height: 149252
   Height (verified): 149253

✨ Successfully connected to Ciphera node at 63.176.138.198:8091
```

---

### 2. Mint Tokens

Minting brings tokens from Citrea into the private rollup. Your wallet we need to have wcBTC for minting into the contract and cBTC for gas. cBTC faucet is found [here](https://citrea.xyz/faucet). 

#### Step 2a: Approve ERC20 Spending

First, approve the rollup contract to spend your wcBTC tokens:

```bash
cast send 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  "approve(address,uint256)" \
  0x1a0E789aa9aE8883C5e55B8E57EFC94F00943d53 \
  100000000000000000000 \
  --rpc-url https://rpc.testnet.citrea.xyz \
  --private-key <YOUR_CITREA_PRIVATE_KEY>
```

> **Note:** You need `cast` from [Foundry](https://book.getfoundry.sh/getting-started/installation)

#### Step 2b: Mint to Your Wallet

```bash
./target/release/ciphera-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  --chain 5115 \
  --token 0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93 \
  mint \
  --secret <YOUR_CITREA_SECRET_KEY> \
  --geth-rpc https://rpc.testnet.citrea.xyz \
  --amount 100000000000000
```

**Parameters:**
- `--secret`: Your wallet's private key (32-byte hex)
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

Transfer tokens privately to another user. This is done via creating a note and sending the note to a user via a communication channel (i.e. Signal).

#### Step 3a: Create a Note for the Recipient

```bash
./target/release/ciphera-cli \
  --name alice \
  --host 63.176.138.198 \
  --port 8091 \
  spend \
  --amount 100000000000000
  alice-note.json
```

**What happens:**
- Alice's wallet is loaded
- A ZK proof is generated (~5-10 seconds)
- A transaction is submitted to the network
- A note file is created: `alice-note.json`

---

#### Step 3b (if needed): Create Bob's wallet

```bash
./target/release/ciphera-cli \
  --name bob \
  --host 63.176.138.198 \
  --port 8091 \
  connect
```
---

#### Step 3c: Share the Note

If you are running the test by yourself, simply create a wallet for Bob in another terminal view. If sending to another person, transfer the note file via a messaging application (i.e. Signal).

**Securely share `alice-note.json` with the recipient** (Bob). This file contains:
- The encrypted note details
- The value
- The secret key to claim it

> ⚠️ **Security:** Anyone with this file can claim the funds! Share it securely.

---

### 4. Receive Tokens

Claim tokens sent to you from a note file.

```bash
./target/release/ciphera-cli \
  --name bob \
  --host 63.176.138.198 \
  --port 8091 \
  receive \
  --note alice-note.json
```

**Expected Output:**
```
🗝 Found note file!
⏳ Generating proof... (this may take 10-30 seconds)
✅ Transaction <hash> has been sent!
   Height: <block>
   Root hash: <new_root>
```

---

### 5. Check balances

In addition to querying the chain (actions below), you can check your wallet balance with:

```bash
cat <WALLET_NAME>.json
```

---

## Querying the Blockchain

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
