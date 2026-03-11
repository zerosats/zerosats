/**
 * CIPHERA WALLET - Electron Preload Script
 * 
 * This script runs in a sandboxed environment before the renderer loads.
 * It exposes a safe API to the renderer process via contextBridge.
 */

const { contextBridge, ipcRenderer } = require('electron');

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
     * @param {string} [host='ciphera.satsbridge.com'] - Node host
     * @param {number} [port=8091] - Node port
     */
    connect: async (name, host = 'ciphera.satsbridge.com', port = 8091) => {
        const args = [
            '--name', name,
            '--host', host,
            '--port', String(port),
            'sync'
        ];
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
     * @param {string} [params.host='ciphera.satsbridge.com'] - Node host
     * @param {number} [params.port=8091] - Node port
     */
    mint: async (params) => {
        const args = [
            '--name', params.name,
            '--host', params.host || 'ciphera.satsbridge.com',
            '--port', String(params.port || 8091),
            'mint',
            '--geth-rpc', params.gethRpc || 'https://rpc.testnet.citrea.xyz',
            '--secret', params.secret,
            '--amount', String(params.amount),
        ];
        if (params.ticker) {
            args.push('--ticker', params.ticker);
        }
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Spend (create a note for sending)
     * @param {string} name - Wallet name
     * @param {string|number} amount - Amount to spend in wei
     * @param {string} [ticker='WCBTC'] - Token ticker
     */
    spend: async (name, amount, ticker) => {
        const args = [
            '--name', name,
            'spend',
            '--amount', String(amount),
        ];
        if (ticker) {
            args.push('--ticker', ticker);
        }
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Send tokens directly to a Ciphera address
     * @param {string} name - Wallet name
     * @param {string} address - Recipient Ciphera address
     * @param {string} [host='ciphera.satsbridge.com'] - Node host
     * @param {number} [port=8091] - Node port
     */
    spendTo: async (name, address, host = 'ciphera.satsbridge.com', port = 8091) => {
        const args = [
            '--name', name,
            '--host', host,
            '--port', String(port),
            'spend-to',
            '--address', address,
        ];
        return ipcRenderer.invoke('cli:run', args);
    },

    /**
     * Import a note file into the wallet
     * @param {string} name - Wallet name
     * @param {string} notePath - Path to the note JSON file
     */
    importNote: async (name, notePath) => {
        const args = [
            '--name', name,
            'import',
            '--note', notePath,
        ];
        return ipcRenderer.invoke('cli:run', args);
    },
    
    /**
     * Receive a note
     * @param {object} params
     * @param {string} params.name - Wallet name
     * @param {string} [params.noteFile] - Path to note file
     * @param {string} [params.link] - Note link
     * @param {string} [params.host='ciphera.satsbridge.com'] - Node host
     * @param {number} [params.port=8091] - Node port
     */
    receive: async (params) => {
        const args = [
            '--name', params.name,
            '--host', params.host || 'ciphera.satsbridge.com',
            '--port', String(params.port || 8091),
            'receive',
        ];
        
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
     * @param {string} [params.host='ciphera.satsbridge.com'] - Node host
     * @param {number} [params.port=8091] - Node port
     */
    burn: async (params) => {
        const args = [
            '--name', params.name,
            '--host', params.host || 'ciphera.satsbridge.com',
            '--port', String(params.port || 8091),
            'burn',
            '--address', params.address,
            '--amount', String(params.amount),
        ];
        if (params.ticker) {
            args.push('--ticker', params.ticker);
        }
        return ipcRenderer.invoke('cli:run', args);
    },
});

console.log('[Preload] Ciphera API exposed to renderer');
