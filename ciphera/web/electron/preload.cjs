/**
 * CIPHERA WALLET - Electron Preload Script
 * 
 * This script runs in a sandboxed environment before the renderer loads.
 * It exposes a safe API to the renderer process via contextBridge.
 */

const { contextBridge, ipcRenderer } = require('electron');

function appendNodeConnectionArgs(args, { host, port, chain }) {
    // host should include the scheme prefix (e.g. "https://…" or "http://…").
    // TLS is inferred from the scheme by the CLI, so no --no-tls flag is needed.
    args.push('--host', host || 'https://ciphera.satsbridge.com');
    args.push('--port', String(port || 443));
    if (chain) args.push('--chain', String(chain));
}

// Expose protected methods to the renderer process
contextBridge.exposeInMainWorld('ciphera', {
    // ==========================================================================
    // CLI COMMAND EXECUTION
    // ==========================================================================
    
    /**
     * Run a CLI command
     * @param {string[]} args - Command arguments (e.g., ['--name', 'alice', 'create'])
     * @returns {Promise<{success: boolean, output: string, stderr: string, code: number}>}
     */
    runCommand: (args) => ipcRenderer.invoke('cli:run', args),
    
    /**
     * Check if CLI binary exists
     * @returns {Promise<{exists: boolean, path: string, cwd: string}>}
     */
    checkCli: () => ipcRenderer.invoke('cli:check'),
    
    /**
     * Listen for real-time stdout from CLI
     * @param {function} callback - Called with each stdout chunk
     */
    onStdout: (callback) => {
        ipcRenderer.on('cli:stdout', (_, data) => callback(data));
    },
    
    /**
     * Listen for real-time stderr from CLI
     * @param {function} callback - Called with each stderr chunk
     */
    onStderr: (callback) => {
        ipcRenderer.on('cli:stderr', (_, data) => callback(data));
    },
    
    /**
     * Remove stdout listener
     */
    offStdout: () => {
        ipcRenderer.removeAllListeners('cli:stdout');
    },
    
    /**
     * Remove stderr listener
     */
    offStderr: () => {
        ipcRenderer.removeAllListeners('cli:stderr');
    },
    
    // ==========================================================================
    // WALLET FILE OPERATIONS
    // ==========================================================================
    
    /**
     * Read a wallet file
     * @param {string} name - Wallet name (without .json extension)
     * @returns {Promise<{exists: boolean, wallet?: object}>}
     */
    readWallet: (name) => ipcRenderer.invoke('wallet:read', name),
    
    /**
     * List all wallet files
     * @returns {Promise<{wallets: string[]}>}
     */
    listWallets: () => ipcRenderer.invoke('wallet:list'),
    
    /**
     * Read a note file
     * @param {string} name - Wallet name (note file is {name}-note.json)
     * @returns {Promise<{exists: boolean, note?: object, path?: string}>}
     */
    readNote: (name) => ipcRenderer.invoke('note:read', name),
    
    /**
     * Save note to Downloads folder
     * @param {string} name - Wallet name (used in filename)
     * @param {object} noteContent - The note object to save
     * @returns {Promise<{success: boolean, path?: string, filename?: string, error?: string}>}
     */
    saveNoteToDownloads: (name, noteContent) => ipcRenderer.invoke('note:saveToDownloads', { name, noteContent }),
    
    // ==========================================================================
    // DIALOGS
    // ==========================================================================
    
    /**
     * Open file dialog for importing
     * @returns {Promise<{canceled: boolean, filePath?: string}>}
     */
    openFileDialog: () => ipcRenderer.invoke('dialog:openFile'),

    /**
     * Import a wallet JSON file from an arbitrary path into the app working directory
     * @param {string} filePath - Absolute path to the wallet JSON file
     * @returns {Promise<{success: boolean, wallet?: object, error?: string}>}
     */
    importWallet: (filePath) => ipcRenderer.invoke('wallet:import', filePath),

    /**
     * Export the active wallet to a user-chosen file location
     * @param {string} name - Wallet name (without .json extension)
     * @returns {Promise<{success: boolean, canceled?: boolean, path?: string, error?: string}>}
     */
    exportWallet: (name) => ipcRenderer.invoke('wallet:export', name),
    
    // ==========================================================================
    // APP INFO
    // ==========================================================================
    
    /**
     * Get app information
     * @returns {Promise<{version: string, name: string, isDev: boolean, platform: string}>}
     */
    getAppInfo: () => ipcRenderer.invoke('app:info'),
    
    // ==========================================================================
    // CONVENIENCE METHODS FOR SPECIFIC COMMANDS
    // ==========================================================================
    
    /**
     * Create a new wallet
     * @param {string} name - Wallet name
     */
    createWallet: async (name) => {
        const args = ['--name', name, 'create'];
        return ipcRenderer.invoke('cli:run', args);
    },
    
    /**
     * Connect to a node
     * @param {string} name - Wallet name
     * @param {string} [host='https://ciphera.satsbridge.com'] - Node host (include scheme: https:// or http://)
     * @param {number} [port=443] - Node port
     */
    connect: async (name, host = 'https://ciphera.satsbridge.com', port = 443) => {
        const args = ['--name', name];
        appendNodeConnectionArgs(args, { host, port });
        args.push('sync');
        return ipcRenderer.invoke('cli:run', args);
    },
    
    /**
     * Mint tokens
     * @param {object} params
     * @param {string} params.name - Wallet name
     * @param {string} params.amount - Amount in wei
     * @param {string} params.secret - Citrea private key
     * @param {string} [params.gethRpc='https://rpc.testnet.citrea.xyz'] - RPC URL
     * @param {string} [params.ticker='WCBTC'] - Token ticker
     * @param {string} [params.host='https://ciphera.satsbridge.com'] - Node host (include scheme)
     * @param {number} [params.port=443] - Node port
     * @param {number} [params.chain] - Chain ID (from /v0/network)
     * @param {string} [params.rollup] - Rollup contract address (from /v0/network)
     */
    mint: async (params) => {
        const args = ['--name', params.name];
        appendNodeConnectionArgs(args, params);
        if (params.rollup) args.push('--rollup', params.rollup);
        args.push(
            'mint',
            '--geth-rpc', params.gethRpc || 'https://rpc.testnet.citrea.xyz',
            '--secret', params.secret,
            '--amount', String(params.amount),
        );
        if (params.ticker) args.push('--ticker', params.ticker);
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Spend (create a note for sending)
     * @param {string} name - Wallet name
     * @param {string|number} amount - Amount to spend in wei
     * @param {string} [ticker='WCBTC'] - Token ticker
     * @param {number} [chain] - Chain ID (from /v0/network)
     * @param {string} [host='https://ciphera.satsbridge.com'] - Node host (include scheme)
     * @param {number} [port=443] - Node port
     */
    spend: async (name, amount, ticker, chain, host, port) => {
        const args = ['--name', name];
        appendNodeConnectionArgs(args, { host, port, chain });
        args.push('spend', '--amount', String(amount));
        if (ticker) args.push('--ticker', ticker);
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Send tokens directly to a Ciphera address
     * @param {string} name - Wallet name
     * @param {string} address - Recipient Ciphera address
     * @param {string} [host='https://ciphera.satsbridge.com'] - Node host (include scheme)
     * @param {number} [port=443] - Node port
     * @param {number} [chain] - Chain ID (from /v0/network)
     */
    spendTo: async (
        name,
        address,
        host = 'https://ciphera.satsbridge.com',
        port = 443,
        chain,
    ) => {
        const args = ['--name', name];
        appendNodeConnectionArgs(args, { host, port, chain });
        args.push('spend-to', '--address', address);
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Import a note file into the wallet
     * @param {string} name - Wallet name
     * @param {string} notePath - Path to the note JSON file
     * @param {number} [chain] - Chain ID (from /v0/network)
     */
    importNote: async (name, notePath, chain) => {
        const args = ['--name', name];
        if (chain) args.push('--chain', String(chain));
        args.push('import', '--note', notePath);
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Receive a note
     * @param {object} params
     * @param {string} params.name - Wallet name
     * @param {string} [params.noteFile] - Path to note file
     * @param {string} [params.link] - Note link
     * @param {string} [params.host='https://ciphera.satsbridge.com'] - Node host (include scheme)
     * @param {number} [params.port=443] - Node port
     * @param {number} [params.chain] - Chain ID (from /v0/network)
     */
    receive: async (params) => {
        const args = ['--name', params.name];
        appendNodeConnectionArgs(args, params);
        args.push('receive');

        if (params.noteFile) {
            args.push('--note', params.noteFile);
        } else if (params.link) {
            args.push('--link', params.link);
        }

        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Burn tokens back to EVM
     * @param {object} params
     * @param {string} params.name - Wallet name
     * @param {string} params.amount - Amount to burn (in wei)
     * @param {string} params.address - EVM address to receive
     * @param {string} [params.ticker='WCBTC'] - Token ticker
     * @param {string} [params.host='https://ciphera.satsbridge.com'] - Node host (include scheme)
     * @param {number} [params.port=443] - Node port
     * @param {number} [params.chain] - Chain ID (from /v0/network)
     */
    burn: async (params) => {
        const args = ['--name', params.name];
        appendNodeConnectionArgs(args, params);
        args.push('burn', '--address', params.address, '--amount', String(params.amount));
        if (params.ticker) args.push('--ticker', params.ticker);
        return ipcRenderer.invoke('cli:run', args);
    },
});

console.log('[Preload] Ciphera API exposed to renderer');
