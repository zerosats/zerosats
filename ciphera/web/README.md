# Ciphera Wallet - Electron App

A desktop wallet UI for Ciphera private payments network.

## Architecture

This Electron app wraps the `ciphera-cli` binary, providing a beautiful terminal UI while the CLI handles all the heavy lifting:

```
┌─────────────────────────────────────┐
│         Ciphera Electron App        │
│                                     │
│   ┌───────────────────────────┐     │
│   │    Terminal UI (HTML/JS)  │     │
│   │                           │     │
│   │  Your beautiful terminal  │     │
│   │  interface runs here      │     │
│   └───────────────────────────┘     │
│              │                      │
│              │ IPC (Electron)       │
│              ▼                      │
│   ┌───────────────────────────┐     │
│   │     ciphera-cli binary    │     │
│   │                           │     │
│   │  - ZK proof generation    │     │
│   │  - Wallet management      │     │
│   │  - Node communication     │     │
│   │  - Citrea bridge          │     │
│   └───────────────────────────┘     │
└─────────────────────────────────────┘
```

## Prerequisites

1. **Build the CLI binary first:**
   ```bash
   # From the repo root
   cargo build --release
   ```

2. **Install Node.js dependencies:**
   ```bash
   cd web
   npm install
   ```

## Development

Run in development mode (with hot reload):

```bash
npm run electron:dev
```

This will:
1. Start the Vite dev server on port 3000
2. Wait for it to be ready
3. Launch Electron pointing at the dev server
4. Open DevTools automatically

## Production Build

### Build for your current platform:
```bash
npm run electron:build
```

### Build for specific platforms:

**macOS:**
```bash
npm run electron:build:mac
```

**Linux:**
```bash
npm run electron:build:linux
```

Built apps will be in the `release/` directory.

## Project Structure

```
web/
├── electron/
│   ├── main.js        # Electron main process
│   └── preload.js     # IPC bridge (exposes window.ciphera)
├── src/
│   ├── main.js        # UI app logic
│   └── terminal.js    # Terminal component
├── css/
│   └── terminal.css   # Cypherpunk styling
├── build/             # App icons
├── index.html         # Main HTML
├── package.json       # Electron + build config
└── vite.config.js     # Vite bundler config
```

## API Reference

The `window.ciphera` API is exposed to the renderer process:

```javascript
// Create a wallet
await window.ciphera.createWallet('alice');

// Connect to node
await window.ciphera.connect('alice', 'ciphera.satsbridge.com', 8091);

// Mint tokens
await window.ciphera.mint({
    name: 'alice',
    amount: '100000000000000',
    secret: '0x...',
    gethRpc: 'https://rpc.testnet.citrea.xyz'
});

// Spend (create note)
await window.ciphera.spend('alice', '100000000000000');

// Receive note
await window.ciphera.receive({
    name: 'alice',
    noteFile: 'bob-note.json'
});

// Burn back to EVM
await window.ciphera.burn({
    name: 'alice',
    amount: '100000000000000',
    address: '0x...',
    secret: '0x...'
});

// Read wallet file
const wallet = await window.ciphera.readWallet('alice');

// List all wallets
const { wallets } = await window.ciphera.listWallets();

// Run raw CLI command
const result = await window.ciphera.runCommand(['--name', 'alice', 'sync']);
```

## Troubleshooting

### "CLI binary not found"

Make sure you've built the CLI:
```bash
cargo build --release
```

The app looks for the binary at:
- Development: `../target/release/ciphera-cli`
- Production: Bundled in app resources

### Port 3000 already in use

The dev server uses port 3000. Kill any existing process:
```bash
lsof -i :3000 | grep LISTEN | awk '{print $2}' | xargs kill
```

### Electron window is blank

Check the DevTools console for errors. Common issues:
- Vite dev server not running
- Missing dependencies

## License

MIT



