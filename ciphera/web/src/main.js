/**
 * CIPHERA WALLET - Electron UI
 *
 * This is the main entry point for the Ciphera wallet UI.
 * It communicates with the CLI binary through Electron's IPC bridge.
 *
 * The window.ciphera API is exposed by electron/preload.js
 */

import {Terminal} from './terminal.js';
import {ethers} from 'ethers';

// Contract addresses for Citrea testnet
const CONTRACTS = {
    WCBTC: '0x8d0c9d1c17aE5e40ffF9bE350f57840E9E66Cd93',
    USDC: '0x52f74a8f9bdd29f77a5efd7f6cb44dcf6906a4b6',
};

// ERC20 ABI (just what we need for approve/allowance)
const ERC20_ABI = [
    'function approve(address spender, uint256 amount) returns (bool)',
    'function allowance(address owner, address spender) view returns (uint256)',
    'function balanceOf(address account) view returns (uint256)',
];

class CipheraApp {
    constructor() {
        // Wait for DOM
        if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', () => this.setup());
        } else {
            this.setup();
        }
    }

    setup() {
        // Setup version
        document.getElementById('appVersion').textContent = `v${__APP_VERSION__}`;

        // Initialize terminal
        this.terminal = new Terminal(document.getElementById('terminalOutput'));

        // App state
        this.state = {
            walletName: null,
            walletAddress: null,
            balance: 0,
            connected: false,
            cliReady: false,
            pendingNote: null, // Stores note waiting to be saved
            nodeHost: null,
            nodePort: null,
            chainId: null,
            rollupContract: null,
        };

        // Prompt system state
        this.promptState = {
            active: false,
            command: null,
            prompts: [],
            currentIndex: 0,
            answers: {},
            isPassword: false,
        };

        // Current view
        this.currentView = 'wallet';

        // Explorer auto-refresh interval
        this.explorerPollInterval = null;

        // Get DOM elements
        this.elements = {
            walletName: document.getElementById('walletName'),
            walletAddress: document.getElementById('walletAddress'),
            walletBalance: document.getElementById('walletBalance'),
            balanceFill: document.getElementById('balanceFill'),
            nodeEndpoint: document.getElementById('nodeEndpoint'),
            nodeStatusIndicator: document.getElementById('nodeStatusIndicator'),
            nodeStatusLabel: document.getElementById('nodeStatusLabel'),
            connectionStatus: document.getElementById('connectionStatus'),
            statusText: document.getElementById('statusText'),
            commandInput: document.getElementById('commandInput'),
            settingsDropdown: document.getElementById('settingsDropdown'),
            settingsBtn: document.getElementById('settingsBtn'),
            dropdownMenu: document.getElementById('dropdownMenu'),
            walletTab: document.getElementById('walletTab'),
            explorerTab: document.getElementById('explorerTab'),
            walletView: document.getElementById('walletView'),
            explorerView: document.getElementById('explorerView'),
            explorerInput: document.getElementById('explorerInput'),
            statHeight: document.getElementById('statHeight'),
            statTxCount: document.getElementById('statTxCount'),
            statElements: document.getElementById('statElements'),
            resultTitle: document.getElementById('resultTitle'),
            resultsPlaceholder: document.getElementById('resultsPlaceholder'),
            jsonOutput: document.getElementById('jsonOutput'),
            copyJsonBtn: document.getElementById('copyJsonBtn'),
        };

        this.bindEvents();
        this.init();
    }

    async init() {
        // Check if running in Electron
        if (typeof window.ciphera === 'undefined') {
            this.terminal.log('⚠ Not running in Electron - CLI features disabled', 'warning');
            this.terminal.log('  Run with: npm run electron:dev', 'dim');
            return;
        }

        // Check CLI binary
        try {
            const cliCheck = await window.ciphera.checkCli();
            if (cliCheck.exists) {
                this.state.cliReady = true;
            } else {
                this.terminal.log('✗ CLI binary not found', 'error');
                this.terminal.log('  Build with: cargo build --release', 'dim');
                return;
            }
        } catch (e) {
            this.terminal.log(`✗ CLI check failed: ${e.message}`, 'error');
            return;
        }

        // CLI output is suppressed for clean UX - we show our own status messages
        // Errors are logged to console for debugging
        window.ciphera.onStdout(() => {
        });
        window.ciphera.onStderr((data) => {
            console.warn('CLI stderr:', data);
        });

        // Check for existing wallets
        let hasWallet = false;
        try {
            const result = await window.ciphera.listWallets();
            hasWallet = result.wallets && result.wallets.length > 0;
            if (hasWallet) {
                await this.loadSavedWallet();
            }
        } catch (e) {
            console.error('Failed to check wallets:', e);
        }

        // Show appropriate welcome message
        if (hasWallet) {
            this.showReturningUserMessage();
        } else {
            this.showWelcomeMessage();
        }

        this.updateTime();
        setInterval(() => this.updateTime(), 1000);
    }

    showWelcomeMessage() {
        this.terminal.separator('═');
        this.terminal.log('   WELCOME TO CIPHERA - Private Bitcoin Payments', 'info');
        this.terminal.separator('═');
        this.terminal.log('', 'default');
        this.terminal.log('Ciphera lets you make completely private Bitcoin payments', 'default');
        this.terminal.log('using zero-knowledge proofs.', 'default');
        this.terminal.log('', 'default');
        this.terminal.log('HOW IT WORKS:', 'info');
        this.terminal.log('  [1] CREATE  → Generate a new encrypted wallet', 'default');
        this.terminal.log('  [2] CONNECT → Connect to the Ciphera network', 'default');
        this.terminal.log('  [3] MINT    → Deposit wcBTC from Citrea into Ciphera', 'default');
        this.terminal.log('  [4] SEND    → Send private payments (generates a note)', 'default');
        this.terminal.log('  [5] RECEIVE → Claim payments sent to you', 'default');
        this.terminal.log('  [6] BURN    → Withdraw back to Citrea', 'default');
        this.terminal.log('', 'default');
        this.terminal.log('IMPORTANT:', 'warning');
        this.terminal.log('  To send payments, transfer the note file to the', 'default');
        this.terminal.log('  recipient over a secure channel (e.g. Signal)', 'default');
        this.terminal.log('', 'default');
        this.terminal.separator();
        this.terminal.log('GETTING STARTED:', 'info');
        this.terminal.log('  Type "create" to create your wallet', 'success');
        this.terminal.log('', 'default');
        this.terminal.log('PREREQUISITES FOR MINTING:', 'info');
        this.terminal.log('  • wcBTC tokens on Citrea testnet', 'dim');
        this.terminal.log('  • Connect to a Ciphera node first so the app can fetch', 'dim');
        this.terminal.log('    the active rollup contract for approvals', 'dim');
        this.terminal.separator('═');
    }

    showReturningUserMessage() {
        this.terminal.log(`✓ ${this.state.walletName} | ${this.formatBalance(this.state.balance)} wcBTC`, 'success');
        this.terminal.separator();
        this.terminal.log('COMMANDS:', 'info');
        this.terminal.log('  [1] CREATE   [2] CONNECT   [3] MINT', 'default');
        this.terminal.log('  [4] SEND     [5] RECEIVE   [6] BURN', 'default');
        this.terminal.separator();
        this.terminal.log('To perform an action, enter a number or command + Enter', 'dim');
    }

    bindEvents() {
        // Command input
        if (this.elements.commandInput) {
            this.elements.commandInput.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') {
                    e.preventDefault();
                    const input = this.elements.commandInput.value.trim();
                    this.elements.commandInput.value = '';

                    if (this.promptState.active) {
                        this.handlePromptAnswer(input);
                    } else {
                        this.handleCommand(input);
                    }
                } else if (e.key === 'Escape' && this.promptState.active) {
                    this.cancelPrompt();
                }
            });
        }

        // Action buttons
        document.querySelectorAll('.action-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                const action = btn.dataset.action;
                this.handleAction(action);
            });
        });

        // Settings dropdown
        if (this.elements.settingsBtn) {
            this.elements.settingsBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleSettingsDropdown();
            });
        }

        // Dropdown items
        document.querySelectorAll('.dropdown-item').forEach(item => {
            item.addEventListener('click', () => {
                const action = item.dataset.action;
                if (action) {
                    this.handleSettingsAction(action);
                }
                this.closeSettingsDropdown();
            });
        });

        // Close dropdown on outside click
        document.addEventListener('click', () => this.closeSettingsDropdown());

        // Tab switching
        if (this.elements.walletTab) {
            this.elements.walletTab.addEventListener('click', () => this.showView('wallet'));
        }
        if (this.elements.explorerTab) {
            this.elements.explorerTab.addEventListener('click', () => this.showView('explorer'));
        }

        // Explorer search
        if (this.elements.explorerInput) {
            this.elements.explorerInput.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') {
                    e.preventDefault();
                    this.handleExplorerSearch(this.elements.explorerInput.value.trim());
                }
            });
        }

    }

    // === Status Update Methods (clean terminal output) ===

    // Show a status message that updates in place
    updateStatus(message, type = 'system') {
        this.terminal.updateLast(message, type);
    }

    // Complete an operation - clear the updating line and show final result
    completeStatus(success, message) {
        this.terminal.clearUpdatable();
        this.terminal.log(success ? `✓ ${message}` : `✗ ${message}`, success ? 'success' : 'error');
    }

    // === Wallet Management ===

    async loadSavedWallet() {
        try {
            const result = await window.ciphera.listWallets();
            if (result.wallets && result.wallets.length > 0) {
                const name = result.wallets[0];
                const walletData = await window.ciphera.readWallet(name);

                if (walletData.exists && walletData.wallet) {
                    this.state.walletName = walletData.wallet.name || name;
                    this.state.walletAddress = walletData.wallet.pk || '---';
                    this.state.balance = walletData.wallet.balance || 0;
                    this.updateWalletDisplay();
                }
            }
        } catch (e) {
            console.error('Failed to load wallet:', e);
        }
    }

    updateWalletDisplay() {
        if (this.elements.walletName) {
            this.elements.walletName.textContent = this.state.walletName || 'NO WALLET';
        }
        if (this.elements.walletAddress) {
            const addr = this.state.walletAddress;
            this.elements.walletAddress.textContent = addr && addr !== '---'
                ? this.truncateAddress(addr)
                : '---';
        }
        this.updateBalanceDisplay();
    }

    updateBalanceDisplay() {
        const formatted = this.formatBalance(this.state.balance);
        if (this.elements.walletBalance) {
            this.elements.walletBalance.textContent = `${formatted} wcBTC`;
        }
        if (this.elements.balanceFill) {
            const percent = Math.min(100, (this.state.balance / 1e18) * 100);
            this.elements.balanceFill.style.width = `${percent}%`;
        }
    }

    updateConnectionStatus(connected, endpoint = null) {
        this.state.connected = connected;
        if (!connected) {
            this.state.nodeHost = null;
            this.state.nodePort = null;
            // Stop explorer polling when disconnected
            if (this.explorerPollInterval) {
                clearInterval(this.explorerPollInterval);
                this.explorerPollInterval = null;
            }
        }

        if (this.elements.nodeStatusIndicator) {
            this.elements.nodeStatusIndicator.className = 'status-indicator ' + (connected ? 'connected' : '');
        }
        if (this.elements.nodeStatusLabel) {
            this.elements.nodeStatusLabel.textContent = connected ? 'CONNECTED' : 'DISCONNECTED';
        }
        if (this.elements.connectionStatus) {
            this.elements.connectionStatus.className = 'status-dot ' + (connected ? 'connected' : '');
        }
        if (this.elements.statusText) {
            this.elements.statusText.textContent = connected ? 'CONNECTED' : 'DISCONNECTED';
        }
        // Update endpoint display
        if (this.elements.nodeEndpoint) {
            this.elements.nodeEndpoint.textContent = connected && endpoint ? endpoint : 'DISCONNECTED';
        }
    }

    truncateAddress(address) {
        if (!address || address.length < 16) return address || '---';
        const str = address.startsWith('0x') ? address : `0x${address}`;
        return `${str.slice(0, 10)}...${str.slice(-8)}`;
    }

    formatBalance(balance) {
        const btc = Number(balance) / 1e18;
        return btc.toFixed(8);
    }

    // Convert wcBTC (decimal) to wei for CLI
    wcbtcToWei(wcbtc) {
        // Multiply by 10^18, handle decimals carefully
        const parts = wcbtc.toString().split('.');
        const whole = parts[0] || '0';
        const decimal = (parts[1] || '').padEnd(18, '0').slice(0, 18);
        return whole + decimal;
    }

    updateTime() {
        const timeEl = document.getElementById('currentTime');
        if (timeEl) {
            const now = new Date();
            timeEl.textContent = now.toLocaleTimeString('en-US', {hour12: false});
        }
    }

    // === Command Handling ===

    handleCommand(input) {
        if (!input) return;

        this.terminal.log(`> ${input}`, 'command');

        const [cmd, ...args] = input.toLowerCase().split(' ');

        switch (cmd) {
            case 'help':
                this.showHelp();
                break;
            case '1':
            case 'create':
                this.handleAction('create');
                break;
            case '2':
            case 'connect':
                this.handleAction('sync');
                break;
            case '3':
            case 'mint':
                this.handleAction('mint');
                break;
            case '4':
            case 'send':
            case 'spend':
                this.handleAction('send');
                break;
            case '5':
            case 'receive':
                this.handleAction('receive');
                break;
            case '6':
            case 'burn':
                this.handleAction('burn');
                break;
            case 'balance':
                this.showBalance();
                break;
            case 'status':
                this.showQuickStatus();
                break;
            case 'clear':
            case 'home':
            case 'back':
                this.goHome();
                break;
            case 'save':
                this.saveNoteToDownloads();
                break;
            default:
                // Check for explorer commands: "tx {hash}" or "block {number}"
                if (cmd === 'tx' && parts.length > 1) {
                    this.lookupTransaction(parts[1]);
                } else if (cmd === 'block' && parts.length > 1) {
                    this.lookupBlock(parts[1]);
                } else {
                    this.terminal.log(`Unknown command: ${cmd}. Type "help" for commands.`, 'error');
                }
        }
    }

    handleAction(action) {
        // Clear terminal for fresh start on each action
        this.terminal.clear();

        switch (action) {
            case 'create':
                this.startCreateWallet();
                break;
            case 'connect':
                this.startSync();
                break; // #TODO: refactor. Left for backward compatibility
            case 'sync':
                this.startSync();
                break;
            case 'mint':
                this.startMint();
                break;
            case 'send':
                this.startSend();
                break;
            case 'receive':
                this.startReceive();
                break;
            case 'burn':
                this.startBurn();
                break;
        }
    }

    showHelp() {
        this.terminal.separator();
        this.terminal.log('AVAILABLE COMMANDS:', 'info');
        this.terminal.log('  [1] create   - Create new wallet', 'default');
        this.terminal.log('  [2] connect  - Connect to Ciphera node', 'default');
        this.terminal.log('  [3] mint     - Mint wcBTC from Citrea', 'default');
        this.terminal.log('  [4] send     - Create a note to send', 'default');
        this.terminal.log('  [5] receive  - Receive a note', 'default');
        this.terminal.log('  [6] burn     - Burn tokens back to Citrea', 'default');
        this.terminal.log('', 'default');
        this.terminal.log('EXPLORER:', 'info');
        this.terminal.log('  tx {hash}    - Look up a transaction', 'default');
        this.terminal.log('  block {num}  - Look up a block', 'default');
        this.terminal.log('', 'default');
        this.terminal.log('OTHER:', 'info');
        this.terminal.log('  balance      - Check your balance', 'default');
        this.terminal.log('  status       - Show system status', 'default');
        this.terminal.log('  clear        - Clear terminal', 'default');
        this.terminal.separator();
    }

    showBalance() {
        if (!this.state.walletName) {
            this.terminal.log('No wallet loaded. Run "create" first.', 'warning');
            return;
        }
        this.terminal.log(`Balance: ${this.formatBalance(this.state.balance)} wcBTC`, 'info');
    }

    showQuickStatus() {
        const wallet = this.state.walletName || 'None';
        const node = this.state.connected ? '✓' : '✗';
        const balance = this.formatBalance(this.state.balance);
        this.terminal.log(`${wallet} | ${balance} wcBTC | Node: ${node}`, 'info');
    }

    // === Explorer Methods ===

    async lookupTransaction(hash) {
        if (!this.state.connected || !this.state.nodeHost) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        this.terminal.clear();
        this.terminal.log('TRANSACTION LOOKUP', 'info');
        this.updateStatus(`⏳ Fetching transaction ${hash.slice(0, 16)}...`);

        try {
            const response = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/transactions/${hash}`);

            if (!response.ok) {
                if (response.status === 404) {
                    this.completeStatus(false, 'Transaction not found');
                } else {
                    this.completeStatus(false, `Error: ${response.status}`);
                }
                return;
            }

            const tx = await response.json();
            this.terminal.clearUpdatable();

            this.terminal.log('TRANSACTION', 'success');
            this.terminal.separator('─');
            this.terminal.log(`Hash: ${tx.hash || hash}`, 'default');
            this.terminal.log(`Height: ${tx.height || 'pending'}`, 'default');
            if (tx.block_hash) {
                this.terminal.log(`Block: ${tx.block_hash}`, 'default');
            }
            if (tx.timestamp) {
                const date = new Date(tx.timestamp).toLocaleString();
                this.terminal.log(`Time: ${date}`, 'default');
            }
            if (tx.inputs !== undefined) {
                this.terminal.log(`Inputs: ${tx.inputs}`, 'default');
            }
            if (tx.outputs !== undefined) {
                this.terminal.log(`Outputs: ${tx.outputs}`, 'default');
            }
            this.terminal.separator('─');

        } catch (e) {
            this.completeStatus(false, `Lookup failed: ${e.message}`);
        }
    }

    async lookupBlock(blockNum) {
        if (!this.state.connected || !this.state.nodeHost) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        this.terminal.clear();
        this.terminal.log('BLOCK LOOKUP', 'info');
        this.updateStatus(`⏳ Fetching block ${blockNum}...`);

        try {
            const response = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/blocks/${blockNum}`);

            if (!response.ok) {
                if (response.status === 404) {
                    this.completeStatus(false, 'Block not found');
                } else {
                    this.completeStatus(false, `Error: ${response.status}`);
                }
                return;
            }

            const block = await response.json();
            this.terminal.clearUpdatable();

            this.terminal.log(`BLOCK #${blockNum}`, 'success');
            this.terminal.separator('─');
            if (block.hash) {
                this.terminal.log(`Hash: ${block.hash}`, 'default');
            }
            if (block.height !== undefined) {
                this.terminal.log(`Height: ${block.height}`, 'default');
            }
            if (block.timestamp) {
                const date = new Date(block.timestamp).toLocaleString();
                this.terminal.log(`Time: ${date}`, 'default');
            }
            if (block.parent_hash) {
                this.terminal.log(`Parent: ${block.parent_hash}`, 'default');
            }
            if (block.transactions !== undefined) {
                const txCount = Array.isArray(block.transactions) ? block.transactions.length : block.transactions;
                this.terminal.log(`Transactions: ${txCount}`, 'default');
            }
            if (block.root_hash) {
                this.terminal.log(`Root: ${block.root_hash}`, 'default');
            }
            this.terminal.separator('─');

        } catch (e) {
            this.completeStatus(false, `Lookup failed: ${e.message}`);
        }
    }

    goHome() {
        this.terminal.clear();
        // Cancel any active prompt
        if (this.promptState.active) {
            this.promptState = {
                active: false,
                command: null,
                prompts: [],
                currentIndex: 0,
                answers: {},
                isPassword: false
            };
        }
        // Clear pending note
        this.state.pendingNote = null;
        // Show appropriate home screen
        if (this.state.walletName) {
            this.showReturningUserMessage();
        } else {
            this.showWelcomeMessage();
        }
    }

    async browseForFile() {
        try {
            this.terminal.log('Opening file browser...', 'dim');
            const result = await window.ciphera.openFileDialog();

            if (result.canceled) {
                this.terminal.log('File selection cancelled', 'dim');
                this.showNextPrompt(); // Show the prompt again
                return;
            }

            // Use the selected file path
            const filePath = result.filePath;
            this.terminal.log('Note selected', 'success');

            // Store the answer and continue
            const prompt = this.promptState.prompts[this.promptState.currentIndex];
            this.promptState.answers[prompt.key] = filePath;
            this.promptState.currentIndex++;
            this.showNextPrompt();

        } catch (e) {
            this.terminal.log(`Browse failed: ${e.message}`, 'error');
            this.showNextPrompt();
        }
    }

    async saveNoteToDownloads() {
        if (!this.state.pendingNote) {
            this.terminal.log('No note to save. Create a note first with "send"', 'warning');
            return;
        }

        try {
            this.updateStatus('⏳ Saving note to Downloads...');

            const result = await window.ciphera.saveNoteToDownloads(
                this.state.walletName,
                this.state.pendingNote.content
            );

            if (result.success) {
                this.completeStatus(true, `NOTE SAVED TO DOWNLOADS: ${result.filename}`);
                this.terminal.log('Send this file via Signal to recipient', 'info');
                // Clear the pending note
                this.state.pendingNote = null;
            } else {
                this.completeStatus(false, `SAVE FAILED: ${result.error}`);
            }
        } catch (e) {
            this.completeStatus(false, `SAVE FAILED: ${e}`);
        }
    }

    // === Prompt System ===

    startPromptSequence(command, prompts) {
        this.promptState = {
            active: true,
            command,
            prompts,
            currentIndex: 0,
            answers: {},
            isPassword: false,
        };
        this.showNextPrompt();
    }

    showNextPrompt() {
        if (this.promptState.currentIndex >= this.promptState.prompts.length) {
            this.executePromptCommand();
            return;
        }

        const prompt = this.promptState.prompts[this.promptState.currentIndex];
        this.promptState.isPassword = prompt.type === 'password';

        this.terminal.log(`${prompt.label}`, 'prompt');

        if (this.elements.commandInput) {
            this.elements.commandInput.type = this.promptState.isPassword ? 'password' : 'text';
            this.elements.commandInput.placeholder = prompt.placeholder || '';
            this.elements.commandInput.focus();
        }
    }

    handlePromptAnswer(input) {
        // Check if user wants to go home
        if (input.toLowerCase() === 'clear' || input.toLowerCase() === 'home' || input.toLowerCase() === 'back') {
            this.goHome();
            return;
        }

        // Check if user wants to save a pending note
        if (input.toLowerCase() === 'save') {
            this.saveNoteToDownloads();
            return;
        }

        // Check if user wants to browse for a file
        if (input.toLowerCase() === 'browse') {
            this.browseForFile();
            return;
        }

        const prompt = this.promptState.prompts[this.promptState.currentIndex];

        if (prompt.required && !input && !prompt.default) {
            this.terminal.log('This field is required.', 'error');
            return;
        }

        // Mask password in terminal
        const displayValue = this.promptState.isPassword ? '********' : input;
        this.terminal.log(`  → ${displayValue || prompt.default || '(default)'}`, 'dim');

        this.promptState.answers[prompt.key] = input || prompt.default;
        this.promptState.currentIndex++;

        if (this.elements.commandInput) {
            this.elements.commandInput.type = 'text';
        }

        this.showNextPrompt();
    }

    cancelPrompt() {
        this.terminal.log('Operation cancelled.', 'warning');
        this.resetPromptState();
    }

    resetPromptState() {
        this.promptState = {
            active: false,
            command: null,
            prompts: [],
            currentIndex: 0,
            answers: {},
            isPassword: false,
        };
        if (this.elements.commandInput) {
            this.elements.commandInput.type = 'text';
            this.elements.commandInput.placeholder = 'Enter command or press action key...';
        }
    }

    async executePromptCommand() {
        const {command, answers} = this.promptState;
        this.resetPromptState();

        switch (command) {
            case 'create':
                await this.executeCreateWallet(answers);
                break;
            case 'connect':
                await this.executeSync(answers);
                break; // #TODO: refactor later
            case 'sync':
                await this.executeSync(answers);
                break;
            case 'mint':
                await this.executeMint(answers);
                break;
            case 'send':
                await this.executeSend(answers);
                break;
            case 'receive':
                await this.executeReceive(answers);
                break;
            case 'burn':
                await this.executeBurn(answers);
                break;
        }
    }

    // === Command Implementations (using CLI) ===

    startCreateWallet() {
        this.terminal.log('CREATE WALLET', 'info');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('create', [
            {key: 'name', label: 'Wallet name:', placeholder: 'e.g., alice', required: true}
        ]);
    }

    async executeCreateWallet(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        try {
            this.updateStatus('⏳ CREATE: Generating wallet keys...');
            const result = await window.ciphera.createWallet(answers.name);

            if (!result.success) {
                this.completeStatus(false, `CREATE FAILED - ${result.error}`);
                return;
            }

            // Reload wallet data
            const walletData = await window.ciphera.readWallet(answers.name);
            if (walletData.exists) {
                this.state.walletName = answers.name;
                this.state.walletAddress = walletData.wallet.pk;
                this.state.balance = walletData.wallet.balance || 0;
                this.updateWalletDisplay();
            }

            this.completeStatus(true, `WALLET CREATED: Welcome, ${answers.name}!`);
            this.terminal.log('Next: Connect to node, then mint wcBTC', 'info');

        } catch (e) {
            this.completeStatus(false, `CREATE FAILED: ${e}`);
        }
    }

    startSync() {
        this.terminal.log('CONNECT TO NODE', 'info');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('sync', [
            {key: 'host', label: 'Host:', placeholder: 'ciphera.satsbridge.com', default: 'ciphera.satsbridge.com'},
            {key: 'port', label: 'Port:', placeholder: '443', default: '443'}
        ]);
    }

    async executeSync(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        if (!this.state.walletName) {
            this.terminal.log('Create a wallet first with "create" command.', 'warning');
            return;
        }

        this.updateStatus(`⏳ CONNECT: Reaching ${answers.host}:${answers.port}...`);

        const result = await window.ciphera.connect(
            this.state.walletName,
            answers.host,
            parseInt(answers.port)
        );

        if (!result.success) {
            this.completeStatus(false, `CONNECT FAILED - ${result.error}`);
            return;
        }

        // Store node connection info in state
        this.state.nodeHost = answers.host;
        this.state.nodePort = parseInt(answers.port);

        // Fetch network info to get chain_id and rollup_contract
        try {
            const networkRes = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/network`);
            if (!networkRes.ok) {
                throw new Error(`HTTPs ${networkRes.status}`);
            }
            const network = await networkRes.json();

            // On first connect, store values. On reconnect, verify they haven't changed.
            if (this.state.chainId === null) {
                this.state.chainId = network.chain_id;
                this.terminal.log(`Chain ID: ${network.chain_id}`, 'dim');
            }

            if (this.state.rollupContract === null) {
                this.state.rollupContract = network.rollup_contract;
                this.terminal.log(`Rollup: ${network.rollup_contract}`, 'dim');
            }

            if (network.chain_id !== this.state.chainId) {
                this.completeStatus(false, `DISCONNECT - chain_id mismatch: expected ${this.state.chainId}, got ${network.chain_id}. Create new a wallet for given node`);
                return;
            }

            if (network.rollup_contract.toLowerCase() !== this.state.rollupContract.toLowerCase()) {
                this.completeStatus(false, `DISCONNECT - rollup contract mismatch: expected ${this.state.rollupContract}, got ${network.rollup_contract}. Create a new wallet for given node`);
                return;
            }
        } catch (e) {
            this.terminal.log(`Warning: could not fetch network info: ${e.message}`, 'warning');
        }

        this.updateConnectionStatus(true, `${this.state.nodeHost}:${this.state.nodePort}`);

        this.completeStatus(true, `CONNECTED: ${this.state.nodeHost}:${this.state.nodePort}`);
    }

    startMint() {
        if (!this.state.walletName) {
            this.terminal.log('Create a wallet first (type "1" or "create")', 'warning');
            return;
        }

        if (!this.state.connected) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        this.terminal.log('MINT wcBTC', 'info');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('mint', [
            {key: 'amount', label: 'Amount (wcBTC):', placeholder: 'e.g., 0.00001', required: true},
            {key: 'secret', label: 'Citrea private key:', placeholder: '0x...', type: 'password', required: true},
            {
                key: 'gethRpc',
                label: 'RPC URL:',
                placeholder: 'https://rpc.testnet.citrea.xyz',
                default: 'https://rpc.testnet.citrea.xyz'
            },
        ]);
    }

    async executeMint(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        // Convert wcBTC input to wei for CLI
        const amountWei = this.wcbtcToWei(answers.amount);
        const rollupContract = this.state.rollupContract;

        if (!rollupContract) {
            this.completeStatus(false, 'MINT FAILED - missing rollup contract from node. Reconnect and try again.');
            return;
        }

        let citreaTxHash = null;
        let cipheraTxHash = null;

        // Step 1: Connect and check approval
        this.updateStatus('⏳ MINT: Connecting to Citrea...');
        const provider = new ethers.JsonRpcProvider(answers.gethRpc);
        const wallet = new ethers.Wallet(answers.secret, provider);
        const tokenContract = new ethers.Contract(CONTRACTS.WCBTC, ERC20_ABI, wallet);

        this.updateStatus('⏳ MINT: Checking token approval...');
        const currentAllowance = await tokenContract.allowance(wallet.address, rollupContract);
        const mintAmount = BigInt(amountWei);

        // Step 2: Approve if needed
        if (currentAllowance < mintAmount) {
            this.updateStatus('⏳ MINT: Approving contract...');
            const approveAmount = mintAmount * 10n;
            const tx = await tokenContract.approve(rollupContract, approveAmount);
            citreaTxHash = tx.hash;

            this.updateStatus('⏳ MINT: Waiting for approval confirmation...');
            await tx.wait();
        }

        // Step 3: Generate ZK proof and mint
        this.updateStatus('⏳ MINT: Generating ZK proof (30-60s)...');

        const result = await window.ciphera.mint({
            name: this.state.walletName,
            amount: amountWei,
            secret: answers.secret,
            gethRpc: answers.gethRpc,
            host: this.state.nodeHost,
            port: this.state.nodePort,
            chain: this.state.chainId,
            rollup: rollupContract,
        });

        if (!result.success) {
            this.completeStatus(false, `MINT FAILED - ${result.error}`);
            return;
        }

        // Extract tx hashes from CLI output
        cipheraTxHash = this.extractCipheraTxHash(result.output);
        const citreaMintTxHash = this.extractCitreaMintTxHash(result.output);

        // Step 4: Update wallet balance
        const mintedBalance = this.extractBalance(result.output);
        if (mintedBalance !== null) {
            this.state.balance += mintedBalance;
            this.updateBalanceDisplay();
        } else {
            const walletData = await window.ciphera.readWallet(this.state.walletName);
            if (walletData.exists) {
                this.state.balance = walletData.wallet.balance || 0;
                this.updateBalanceDisplay();
            }
        }

        // Complete!
        this.completeStatus(true, `MINT COMPLETE: +${answers.amount} wcBTC`);

        // Show transaction IDs
        if (citreaTxHash) {
            this.terminal.log(`Citrea Approve TX: ${citreaTxHash}`, 'dim');
        }
        if (citreaMintTxHash) {
            this.terminal.log(`Citrea Mint TX:    ${citreaMintTxHash}`, 'dim');
        }
        if (cipheraTxHash) {
            this.terminal.log(`Ciphera TX:        ${this.padTxHash(cipheraTxHash)}`, 'dim');
        }
    }

    startSend() {
        if (!this.state.walletName) {
            this.terminal.log('Create a wallet first (type "1" or "create")', 'warning');
            return;
        }

        if (!this.state.connected) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        if (this.state.balance <= 0) {
            this.terminal.log('No balance to send. Mint first (type "3" or "mint")', 'warning');
            return;
        }

        const balanceWcbtc = this.formatBalance(this.state.balance);
        this.terminal.log(`SEND (Balance: ${balanceWcbtc} wcBTC)`, 'info');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('send', [
            {
                key: 'amount',
                label: 'Amount (wcBTC):',
                placeholder: `e.g., ${balanceWcbtc}`,
                default: balanceWcbtc,
                required: true
            }
        ]);
    }

    async executeSend(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        // Convert wcBTC input to wei for CLI
        const amountWei = this.wcbtcToWei(answers.amount);

        this.updateStatus('⏳ SEND: Creating private note...');

        const result = await window.ciphera.spend(this.state.walletName, amountWei, undefined, this.state.chainId, this.state.nodeHost, this.state.nodePort);

        if (!result.success) {
            this.completeStatus(false, `SEND FAILED - ${result.error}`);
            return;
        }

        // Check for note file
        const noteData = await window.ciphera.readNote(this.state.walletName);

        // Update wallet balance
        const sentBalance = this.extractBalance(result.output);
        if (sentBalance !== null) {
            this.state.balance = sentBalance;
            this.updateBalanceDisplay();
        } else {
            const walletData = await window.ciphera.readWallet(this.state.walletName);
            if (walletData.exists) {
                this.state.balance = walletData.wallet.balance || 0;
                this.updateBalanceDisplay();
            }
        }

        // Complete!
        if (noteData.exists) {
            this.completeStatus(true, `NOTE CREATED: ${answers.amount} wcBTC`);
            // Store the pending note for saving
            this.state.pendingNote = {
                content: noteData.note,
                amount: answers.amount,
            };
            this.terminal.separator();
            this.terminal.log('Type "save" to save note to Downloads', 'info');
            this.terminal.log('Then send the file via Signal to recipient', 'dim');
        } else {
            this.completeStatus(true, `SEND COMPLETE: -${answers.amount} wcBTC`);
        }
    }

    startReceive() {
        if (!this.state.walletName) {
            this.terminal.log('Create a wallet first (type "1" or "create")', 'warning');
            return;
        }

        if (!this.state.connected) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        this.terminal.log('RECEIVE NOTE', 'info');
        this.terminal.log('Type "browse" to select file from Downloads', 'dim');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('receive', [
            {key: 'noteFile', label: 'Note file path (or "browse"):', placeholder: 'e.g., note.json'},
        ]);
    }

    async executeReceive(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        if (!answers.noteFile) {
            this.terminal.log('Please provide a note file path or type "browse"', 'error');
            return;
        }

        const oldBalance = this.state.balance;

        this.updateStatus('⏳ RECEIVE: Processing note...');

        const result = await window.ciphera.receive({
            name: this.state.walletName,
            noteFile: answers.noteFile,
            host: this.state.nodeHost,
            port: this.state.nodePort,
            chain: this.state.chainId,
        });

        if (!result.success) {
            this.completeStatus(false, `RECEIVE FAILED - ${result.error}`);
            return;
        }

        // Extract Ciphera tx hash from CLI output
        const cipheraTxHash = this.extractCipheraTxHash(result.output);

        // Update wallet balance
        const receivedBalance = this.extractBalance(result.output);
        if (receivedBalance !== null) {
            this.state.balance = receivedBalance;
            this.updateBalanceDisplay();
        } else {
            const walletData = await window.ciphera.readWallet(this.state.walletName);
            if (walletData.exists) {
                this.state.balance = walletData.wallet.balance || 0;
                this.updateBalanceDisplay();
            }
        }

        // Complete!
        const received = this.state.balance - oldBalance;
        const formattedReceived = this.formatBalance(received > 0 ? received : this.state.balance);
        this.completeStatus(true, `RECEIVED: +${formattedReceived} wcBTC`);

        // Show transaction ID
        if (cipheraTxHash) {
            this.terminal.log(`Ciphera TX: ${this.padTxHash(cipheraTxHash)}`, 'dim');
        }
    }

    startBurn() {
        if (!this.state.walletName) {
            this.terminal.log('Create a wallet first (type "1" or "create")', 'warning');
            return;
        }

        if (!this.state.connected) {
            this.terminal.log('Connect to a node first (type "2" or "connect")', 'warning');
            return;
        }

        if (this.state.balance <= 0) {
            this.terminal.log('No balance to burn. Mint first (type "3" or "mint")', 'warning');
            return;
        }

        const balanceWcbtc = this.formatBalance(this.state.balance);
        this.terminal.log(`BURN TO CITREA (Balance: ${balanceWcbtc} wcBTC)`, 'info');
        this.terminal.log('Type "clear" to return home', 'dim');
        this.startPromptSequence('burn', [
            {
                key: 'amount',
                label: 'Amount (wcBTC):',
                placeholder: `e.g., ${balanceWcbtc}`,
                default: balanceWcbtc,
                required: true
            },
            {key: 'address', label: 'EVM address:', placeholder: '0x...', required: true},
        ]);
    }

    async executeBurn(answers) {
        if (!this.state.cliReady) {
            this.terminal.log('CLI not ready', 'error');
            return;
        }

        // Convert wcBTC input to wei for CLI
        const amountWei = this.wcbtcToWei(answers.amount);


        this.updateStatus('⏳ BURN: Withdrawing to Citrea...');

        const result = await window.ciphera.burn({
            name: this.state.walletName,
            amount: amountWei,
            address: answers.address,
            host: this.state.nodeHost,
            port: this.state.nodePort,
            chain: this.state.chainId,
        });

        if (!result.success) {
            this.completeStatus(false, `BURN FAILED - ${result.error}`);
            return;
        }

        // Extract Ciphera tx hash from CLI output
        const cipheraTxHash = this.extractCipheraTxHash(result.output);

        // Update wallet balance
        const burnedBalance = this.extractBalance(result.output);
        if (burnedBalance !== null) {
            this.state.balance = burnedBalance;
            this.updateBalanceDisplay();
        } else {
            const walletData = await window.ciphera.readWallet(this.state.walletName);
            if (walletData.exists) {
                this.state.balance = walletData.wallet.balance || 0;
                this.updateBalanceDisplay();
            }
        }

        // Complete!
        const shortAddress = `${answers.address.slice(0, 8)}...${answers.address.slice(-6)}`;
        this.completeStatus(true, `BURN COMPLETE: ${answers.amount} wcBTC → ${shortAddress}`);

        // Show transaction IDs
        if (cipheraTxHash) {
            this.terminal.log(`Ciphera TX: ${this.padTxHash(cipheraTxHash)}`, 'dim');
        }
    }

    // === TX Hash Helpers ===

    /**
     * Extract Ciphera transaction hash from CLI output
     * CLI prints: "✅ Transaction {hash} has been sent!"
     */
    extractCipheraTxHash(output) {
        if (!output) return null;
        const match = output.match(/Transaction\s+([a-fA-F0-9]+)\s+has been sent/);
        return match ? match[1] : null;
    }

    /**
     * Extract balance from CLI output
     * CLI prints: "Balance {amount} {TICKER}"
     */
    extractBalance(output) {
        if (!output) return null;
        const match = output.match(/Balance\s+(\d+)\s+\w+/);
        return match ? Number(match[1]) : null;
    }

    /**
     * Extract Citrea EVM transaction hash from CLI output
     * CLI prints: "Submitted MINT tx 0x{hash}"
     */
    extractCitreaMintTxHash(output) {
        if (!output) return null;
        const match = output.match(/Submitted MINT tx (0x[a-fA-F0-9]+)/);
        return match ? match[1] : null;
    }


    /**
     * Pad a transaction hash to 64 characters (32 bytes)
     */
    padTxHash(hash) {
        if (!hash) return hash;
        const cleanHash = hash.startsWith('0x') ? hash.slice(2) : hash;
        return cleanHash.padStart(64, '0');
    }

    // === Error Helpers ===
    parseCliError(stderr) {
        const text = stderr || '';

        // Try to find the main error line (after ❌)
        const errorMatch = text.match(/❌\s*([^\n!]+)/);
        if (errorMatch) {
            let msg = errorMatch[1].trim();

            // Also grab the "Error:" line if present
            const detailMatch = text.match(/Error:\s*([^\n]+)/);
            if (detailMatch) {
                const detail = detailMatch[1]
                    .replace(/^undefined error:\s*/i, '')
                    .trim();
                if (detail && !msg.includes(detail)) {
                    msg += `: ${detail}`;
                }
            }

            return msg;
        }

        // Fallback: find any "Error:" line
        const simpleMatch = text.match(/Error:\s*([^\n]+)/);
        if (simpleMatch) {
            return simpleMatch[1].replace(/^undefined error:\s*/i, '').trim();
        }

        return `Process exited with error`;
    }

    // === UI Helpers ===

    toggleSettingsDropdown() {
        if (this.elements.settingsDropdown) {
            this.elements.settingsDropdown.classList.toggle('open');
        }
    }

    closeSettingsDropdown() {
        if (this.elements.settingsDropdown) {
            this.elements.settingsDropdown.classList.remove('open');
        }
    }

    async handleSettingsAction(action) {
        switch (action) {
            case 'export':
                this.terminal.log('Export wallet feature coming soon...', 'info');
                break;
            case 'import':
                await this.importWallet();
                break;
            case 'about':
                this.showAbout();
                break;
        }
    }

    async importWallet() {
        const fileResult = await window.ciphera.openFileDialog();
        if (fileResult.canceled) return;

        this.terminal.separator();
        this.updateStatus('⏳ IMPORT: Reading wallet file...');

        const result = await window.ciphera.importWallet(fileResult.filePath);

        if (!result.success) {
            this.completeStatus(false, `IMPORT FAILED - ${result.error}`);
            return;
        }

        const wallet = result.wallet;
        this.state.walletName = wallet.name;
        this.state.walletAddress = wallet.pk;
        this.state.balance = wallet.balance || 0;
        this.updateWalletDisplay();
        this.updateBalanceDisplay();

        this.completeStatus(true, `WALLET IMPORTED: ${wallet.name}`);
        this.terminal.log(`Address: ${this.truncateAddress(wallet.pk)}`, 'dim');
        this.terminal.log(`Balance: ${this.formatBalance(this.state.balance)} wcBTC`, 'dim');
        this.terminal.log('Connect to a node to sync (type "2" or "connect")', 'info');
    }

    async showAbout() {
        this.terminal.separator();
        this.terminal.log('CIPHERA WALLET', 'info');
        this.terminal.log('Zero-Knowledge Bitcoin Payments', 'dim');
        this.terminal.separator();

        if (window.ciphera) {
            const info = await window.ciphera.getAppInfo();
            this.terminal.log(`Version: ${info.version}`, 'dim');
            this.terminal.log(`Platform: ${info.platform}`, 'dim');
        }

        this.terminal.log('', 'default');
        this.terminal.log('Built with Noir ZK circuits and Barretenberg', 'dim');
        this.terminal.log('https://github.com/zerosats/ciphera-cli', 'dim');
        this.terminal.separator();
    }

    showView(view) {
        this.currentView = view;

        // Update tabs
        if (this.elements.walletTab) {
            this.elements.walletTab.classList.toggle('active', view === 'wallet');
        }
        if (this.elements.explorerTab) {
            this.elements.explorerTab.classList.toggle('active', view === 'explorer');
        }

        // Update views
        if (this.elements.walletView) {
            this.elements.walletView.classList.toggle('hidden', view !== 'wallet');
        }
        if (this.elements.explorerView) {
            this.elements.explorerView.classList.toggle('hidden', view !== 'explorer');
        }

        // Load explorer stats when switching to explorer view
        if (view === 'explorer' && this.state.connected) {
            this.loadExplorerStats();
            // Start auto-refresh every 5 seconds
            if (!this.explorerPollInterval) {
                this.explorerPollInterval = setInterval(() => this.loadExplorerStats(), 5000);
            }
        } else {
            // Stop polling when leaving explorer view
            if (this.explorerPollInterval) {
                clearInterval(this.explorerPollInterval);
                this.explorerPollInterval = null;
            }
        }
    }

    async loadExplorerStats() {
        if (!this.state.nodeHost) return;

        try {
            // Fetch height
            const heightRes = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/height`);
            if (heightRes.ok) {
                const heightData = await heightRes.json();
                if (this.elements.statHeight) {
                    this.elements.statHeight.textContent = heightData.height || heightData;
                }
            }

            // Fetch stats
            const statsRes = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/stats`);
            if (statsRes.ok) {
                const stats = await statsRes.json();
                if (this.elements.statTxCount) {
                    this.elements.statTxCount.textContent = stats.transaction_count || stats.transactions || '0';
                }
                if (this.elements.statElements) {
                    this.elements.statElements.textContent = stats.element_count || stats.elements || '0';
                }
            }
        } catch (e) {
            console.error('Failed to load explorer stats:', e);
        }
    }

    async handleExplorerSearch(query) {
        if (!query) return;

        if (!this.state.connected || !this.state.nodeHost) {
            this.showExplorerError('Connect to a node first (use WALLET tab → connect)');
            return;
        }

        // Clear previous results
        this.showExplorerLoading(query);

        try {
            // Determine search type based on query
            const isNumber = /^\d+$/.test(query);

            if (isNumber) {
                // Block lookup
                await this.explorerLookupBlock(query);
            } else {
                // Try transaction first, then element
                const txResult = await this.explorerLookupTransaction(query);
                if (!txResult) {
                    await this.explorerLookupElement(query);
                }
            }
        } catch (e) {
            this.showExplorerError(`Search failed: ${e.message}`);
        }
    }

    showExplorerLoading(query) {
        if (this.elements.resultsPlaceholder) {
            this.elements.resultsPlaceholder.style.display = 'none';
        }
        if (this.elements.jsonOutput) {
            this.elements.jsonOutput.style.display = 'block';
            this.elements.jsonOutput.textContent = `Searching for ${query}...`;
        }
        if (this.elements.resultTitle) {
            this.elements.resultTitle.textContent = 'SEARCHING...';
        }
        if (this.elements.copyJsonBtn) {
            this.elements.copyJsonBtn.style.display = 'none';
        }
    }

    showExplorerResult(title, data) {
        if (this.elements.resultsPlaceholder) {
            this.elements.resultsPlaceholder.style.display = 'none';
        }
        if (this.elements.jsonOutput) {
            this.elements.jsonOutput.style.display = 'block';
            this.elements.jsonOutput.textContent = JSON.stringify(data, null, 2);
        }
        if (this.elements.resultTitle) {
            this.elements.resultTitle.textContent = title;
        }
        if (this.elements.copyJsonBtn) {
            this.elements.copyJsonBtn.style.display = 'inline-block';
            this.elements.copyJsonBtn.onclick = () => {
                navigator.clipboard.writeText(JSON.stringify(data, null, 2));
                this.elements.copyJsonBtn.textContent = 'COPIED!';
                setTimeout(() => {
                    this.elements.copyJsonBtn.textContent = 'COPY JSON';
                }, 1500);
            };
        }
    }

    showExplorerError(message) {
        if (this.elements.resultsPlaceholder) {
            this.elements.resultsPlaceholder.style.display = 'none';
        }
        if (this.elements.jsonOutput) {
            this.elements.jsonOutput.style.display = 'block';
            this.elements.jsonOutput.textContent = `Error: ${message}`;
        }
        if (this.elements.resultTitle) {
            this.elements.resultTitle.textContent = 'ERROR';
        }
        if (this.elements.copyJsonBtn) {
            this.elements.copyJsonBtn.style.display = 'none';
        }
    }

    async explorerLookupTransaction(hash) {
        // Normalize hash - remove 0x prefix and pad to 64 chars
        const cleanHash = hash.startsWith('0x') ? hash.slice(2) : hash;
        const paddedHash = cleanHash.padStart(64, '0');

        // Try plural endpoint first (v0/transactions/{hash})
        let url = `https://${this.state.nodeHost}:${this.state.nodePort}/v0/transactions/${paddedHash}`;
        console.log('[Explorer] Trying:', url);
        let response = await fetch(url);
        console.log('[Explorer] Response:', response.status);

        // If not found, try singular endpoint (v0/transaction/{hash})
        if (!response.ok && response.status === 404) {
            url = `https://${this.state.nodeHost}:${this.state.nodePort}/v0/transaction/${paddedHash}`;
            console.log('[Explorer] Trying:', url);
            response = await fetch(url);
            console.log('[Explorer] Response:', response.status);
        }

        if (!response.ok) {
            if (response.status === 404) {
                return false; // Not found, try element
            }
            throw new Error(`HTTP ${response.status}`);
        }

        const data = await response.json();
        console.log('[Explorer] Data:', data);
        this.showExplorerResult('TRANSACTION', data);
        return true;
    }

    async explorerLookupBlock(blockNum) {
        const response = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/blocks/${blockNum}`);

        if (!response.ok) {
            if (response.status === 404) {
                this.showExplorerError(`Block ${blockNum} not found`);
                return;
            }
            throw new Error(`HTTP ${response.status}`);
        }

        const data = await response.json();
        this.showExplorerResult(`BLOCK #${blockNum}`, data);
    }

    async explorerLookupElement(hash) {
        // Normalize hash - remove 0x prefix and pad to 64 chars
        const cleanHash = hash.startsWith('0x') ? hash.slice(2) : hash;
        const paddedHash = cleanHash.padStart(64, '0');

        const response = await fetch(`https://${this.state.nodeHost}:${this.state.nodePort}/v0/elements/${paddedHash}`);

        if (!response.ok) {
            if (response.status === 404) {
                this.showExplorerError('Not found (checked transactions and elements)');
                return;
            }
            throw new Error(`HTTP ${response.status}`);
        }

        const data = await response.json();
        this.showExplorerResult('ELEMENT', data);
    }
}

// Initialize app
const app = new CipheraApp();
