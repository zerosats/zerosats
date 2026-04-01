/**
 * CIPHERA WALLET - Electron Main Process
 * 
 * This is the main process that creates the browser window
 * and handles CLI command execution via child processes.
 */

const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// Disable GPU for compatibility
app.disableHardwareAcceleration();

// Prevent GTK from using GtkFileChooserNative (causes cast errors on Linux/GTK3)
if (process.platform === 'linux') {
    process.env.GTK_USE_PORTAL = '0';
}

// Keep a global reference of the window object
let mainWindow = null;

// Determine if we're in development or production
const isDev = !app.isPackaged;

// Get the path to the CLI binary
function getCliPath() {
    if (isDev) {
        // Development: use the compiled binary from target/release
        const devPath = path.join(__dirname, '..', '..', 'target', 'release', 'ciphera-cli');
        if (fs.existsSync(devPath)) {
            return devPath;
        }
        // Fallback to debug build
        const debugPath = path.join(__dirname, '..', '..', 'target', 'debug', 'ciphera-cli');
        if (fs.existsSync(debugPath)) {
            return debugPath;
        }
        // Fallback to system PATH
        return 'ciphera-cli';
    } else {
        // Production: bundled in resources
        const resourcesPath = process.resourcesPath;
        const binPath = path.join(resourcesPath, 'bin', 'ciphera-cli');
        
        // macOS/Linux
        if (fs.existsSync(binPath)) {
            return binPath;
        }

        throw new Error('CLI binary not found in resources');
    }
}

// Get working directory for wallet files
function getWorkingDirectory() {
    if (isDev) {
        return path.join(__dirname, '..', '..');
    }
    // Production: use user's home directory
    return app.getPath('userData');
}

const WALLET_NAME_PATTERN = /^[a-zA-Z0-9_-]+$/;

function isPlainObject(value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isSerializedElement(value) {
    return typeof value === 'string' && value.length > 0;
}

function isPositiveInteger(value) {
    return Number.isInteger(value) && value > 0;
}

function resolveWalletPath(name) {
    if (typeof name !== 'string' || !WALLET_NAME_PATTERN.test(name)) {
        return null;
    }

    const resolvedCwd = path.resolve(getWorkingDirectory());
    const walletPath = path.resolve(resolvedCwd, `${name}.json`);
    const relativeWalletPath = path.relative(resolvedCwd, walletPath);

    if (relativeWalletPath.startsWith('..') || path.isAbsolute(relativeWalletPath)) {
        return null;
    }

    return walletPath;
}

function isSerializedNote(value) {
    return isPlainObject(value)
        && isSerializedElement(value.kind)
        && isSerializedElement(value.contract)
        && isSerializedElement(value.address)
        && isSerializedElement(value.psi)
        && isSerializedElement(value.value);
}

function isSerializedInputNote(value) {
    return isPlainObject(value)
        && isSerializedNote(value.note)
        && isSerializedElement(value.secret_key);
}

function sanitizeInputNote(note) {
    return {
        note: {
            kind: note.note.kind,
            contract: note.note.contract,
            address: note.note.address,
            psi: note.note.psi,
            value: note.note.value,
        },
        secret_key: note.secret_key,
    };
}

function sanitizeInputNoteMap(value) {
    return Object.fromEntries(
        Object.entries(value).map(([ticker, notes]) => [
            ticker,
            notes.map(sanitizeInputNote),
        ]),
    );
}

function normalizeImportedWallet(wallet) {
    const normalizedWallet = {
        pk: wallet.pk,
        keys: [...wallet.keys],
        pending: sanitizeInputNoteMap(wallet.pending),
        avail: sanitizeInputNoteMap(wallet.avail),
        name: wallet.name,
        balance: wallet.balance,
    };

    if (isPositiveInteger(wallet.chain_id)) {
        normalizedWallet.chain_id = wallet.chain_id;
    }

    return normalizedWallet;
}

function validateImportedWallet(wallet) {
    if (!isPlainObject(wallet)) {
        return 'Invalid wallet file';
    }

    if (typeof wallet.name !== 'string' || !wallet.name) {
        return 'Invalid wallet file (missing or malformed name)';
    }

    if (!WALLET_NAME_PATTERN.test(wallet.name)) {
        return 'Invalid wallet name (only alphanumeric, dash, underscore)';
    }

    if (!isSerializedElement(wallet.pk)) {
        return 'Invalid wallet file (missing or malformed pk)';
    }

    if (!Array.isArray(wallet.keys) || !wallet.keys.every(isSerializedElement)) {
        return 'Invalid wallet file (missing or malformed keys)';
    }

    if (
        !isPlainObject(wallet.pending)
        || !Object.values(wallet.pending).every(
            notes => Array.isArray(notes) && notes.every(isSerializedInputNote),
        )
    ) {
        return 'Invalid wallet file (missing or malformed pending notes)';
    }

    if (
        !isPlainObject(wallet.avail)
        || !Object.values(wallet.avail).every(
            notes => Array.isArray(notes) && notes.every(isSerializedInputNote),
        )
    ) {
        return 'Invalid wallet file (missing or malformed available notes)';
    }

    if (!Number.isInteger(wallet.balance) || wallet.balance < 0) {
        return 'Invalid wallet file (missing or malformed balance)';
    }

    if (wallet.chain_id !== undefined && wallet.chain_id !== null && !isPositiveInteger(wallet.chain_id)) {
        return 'Invalid wallet file (missing or malformed chain id)';
    }

    return null;
}

function createWindow() {
    // Create the browser window
    mainWindow = new BrowserWindow({
        width: 1100,
        height: 800,
        minWidth: 800,
        minHeight: 600,
        title: 'CIPHERA // Private Payments',
        backgroundColor: '#0a0a0a',
        webPreferences: {
            preload: path.join(__dirname, 'preload.cjs'),
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: false, // Required for child_process in preload
        },
        // Frameless for custom title bar (optional)
        // frame: false,
        // titleBarStyle: 'hiddenInset',
    });

    // Load the app
    if (isDev) {
        // Development: load from Vite dev server
        mainWindow.loadURL('http://localhost:3000');
        // Open DevTools in development
        mainWindow.webContents.openDevTools();
    } else {
        // Production: load built files
        mainWindow.loadFile(path.join(__dirname, '..', 'dist', 'index.html'));
    }

    // Handle window closed
    mainWindow.on('closed', () => {
        mainWindow = null;
    });

    // Set app name for macOS
    if (process.platform === 'darwin') {
        app.setName('Ciphera');
    }
}

function parseCliError(stderr, stdout, code) {
    const text = stderr || stdout || '';

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

    return `Process exited with code ${code}`;
}

// Create window when app is ready
app.whenReady().then(createWindow);

// Quit when all windows are closed
app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') {
        app.quit();
    }
});

// On macOS, recreate window when dock icon is clicked
app.on('activate', () => {
    if (mainWindow === null) {
        createWindow();
    }
});

// =============================================================================
// CLI COMMAND HANDLERS
// =============================================================================

/**
 * Run a CLI command and return the result
 * Streams output back to the renderer in real-time
 */
ipcMain.handle('cli:run', async (event, args) => {
    return new Promise((resolve, reject) => {
        const cliPath = getCliPath();
        const cwd = getWorkingDirectory();
        
        console.log(`[CLI] Running: ${cliPath} ${args.join(' ')}`);
        console.log(`[CLI] CWD: ${cwd}`);
        
        const proc = spawn(cliPath, args, {
            cwd,
            env: { ...process.env },
        });
        
        let stdout = '';
        let stderr = '';
        
        // Stream stdout to renderer
        proc.stdout.on('data', (data) => {
            const text = data.toString();
            stdout += text;
            // Send real-time output to renderer
            if (mainWindow && !mainWindow.isDestroyed()) {
                mainWindow.webContents.send('cli:stdout', text);
            }
        });
        
        // Stream stderr to renderer
        proc.stderr.on('data', (data) => {
            const text = data.toString();
            stderr += text;
            if (mainWindow && !mainWindow.isDestroyed()) {
                mainWindow.webContents.send('cli:stderr', text);
            }
        });
        
        proc.on('error', (err) => {
            console.error(`[CLI] Process error:`, err);
            resolve({
                success: false,
                error: `Failed to start CLI: ${err.message}`,
                stdout,
                stderr,
            });
        });
        
        proc.on('close', (code) => {
            console.log(`[CLI] Process exited with code ${code}`);
            if (code === 0) {
                resolve({
                    success: true,
                    output: stdout,
                    stderr,
                    code,
                });
            } else {
                resolve({
                    success: false,
                    error: parseCliError(stderr, stdout, code),
                });
            }
        });
    });
});

/**
 * Check if CLI binary exists
 */
ipcMain.handle('cli:check', async () => {
    try {
        const cliPath = getCliPath();
        const exists = fs.existsSync(cliPath);
        return {
            exists,
            path: cliPath,
            cwd: getWorkingDirectory(),
        };
    } catch (e) {
        return {
            exists: false,
            error: e.message,
        };
    }
});

/**
 * Read a wallet file
 */
ipcMain.handle('wallet:read', async (event, name) => {
    try {
        const walletPath = resolveWalletPath(name);
        if (!walletPath) {
            return {
                exists: false,
                error: 'Invalid wallet name',
            };
        }

        if (!fs.existsSync(walletPath)) {
            return { exists: false };
        }

        const content = fs.readFileSync(walletPath, 'utf8');
        const wallet = JSON.parse(content);
        const validationError = validateImportedWallet(wallet);
        if (validationError) {
            return {
                exists: false,
                error: validationError,
            };
        }

        return {
            exists: true,
            wallet: normalizeImportedWallet(wallet),
        };
    } catch (e) {
        return {
            exists: false,
            error: e.message,
        };
    }
});

/**
 * Import a wallet file from an arbitrary path into the working directory
 */
ipcMain.handle('wallet:import', async (event, filePath) => {
    try {
        if (typeof filePath !== 'string' || !filePath) {
            return { success: false, error: 'File not found' };
        }

        const sourcePath = path.resolve(filePath);
        if (!fs.existsSync(sourcePath) || !fs.statSync(sourcePath).isFile()) {
            return { success: false, error: 'File not found' };
        }

        const content = fs.readFileSync(sourcePath, 'utf8');
        const wallet = JSON.parse(content);
        const validationError = validateImportedWallet(wallet);
        if (validationError) {
            return { success: false, error: validationError };
        }

        const destPath = resolveWalletPath(wallet.name);
        if (!destPath) {
            return { success: false, error: 'Invalid wallet destination' };
        }

        if (fs.existsSync(destPath)) {
            return { success: false, error: `Wallet ${wallet.name} already exists` };
        }

        const normalizedWallet = normalizeImportedWallet(wallet);
        fs.writeFileSync(destPath, `${JSON.stringify(normalizedWallet, null, 2)}\n`, 'utf8');

        return { success: true, wallet: normalizedWallet };
    } catch (e) {
        return { success: false, error: e.message };
    }
});

/**
 * List wallet files
 */
ipcMain.handle('wallet:list', async () => {
    try {
        const cwd = getWorkingDirectory();
        const files = fs.readdirSync(cwd);
        const wallets = files
            .filter(f => f.endsWith('.json') && !f.includes('-note'))
            .map(f => f.replace('.json', ''));
        
        return { wallets };
    } catch (e) {
        return { wallets: [], error: e.message };
    }
});

/**
 * Read a note file
 */
ipcMain.handle('note:read', async (event, name) => {
    try {
        const cwd = getWorkingDirectory();
        const notePath = path.join(cwd, `${name}-note.json`);
        
        if (!fs.existsSync(notePath)) {
            return { exists: false };
        }
        
        const content = fs.readFileSync(notePath, 'utf8');
        const note = JSON.parse(content);
        
        return {
            exists: true,
            note,
            path: notePath,
        };
    } catch (e) {
        return {
            exists: false,
            error: e.message,
        };
    }
});

/**
 * Open file dialog for importing notes
 */
ipcMain.handle('dialog:openFile', async () => {
    // GTK_USE_PORTAL=0 is set at startup to prevent GtkFileChooserNative crashes
    // on Linux/GTK3. With that in place, passing mainWindow is safe and ensures
    // the dialog appears in front of the app window instead of behind it.
    const result = await dialog.showOpenDialog(mainWindow, {
        properties: ['openFile'],
        filters: [
            { name: 'JSON Files', extensions: ['json'] },
            { name: 'All Files', extensions: ['*'] },
        ],
    });
    
    if (result.canceled) {
        return { canceled: true };
    }
    
    return {
        canceled: false,
        filePath: result.filePaths[0],
    };
});

/**
 * Get app info
 */
ipcMain.handle('app:info', async () => {
    return {
        version: app.getVersion(),
        name: app.getName(),
        isDev,
        platform: process.platform,
        arch: process.arch,
        userData: app.getPath('userData'),
    };
});

/**
 * Save note to Downloads folder
 */
ipcMain.handle('note:saveToDownloads', async (event, params) => {
    try {
        console.log('[Note] Saving to downloads, params:', params);
        
        const { name, noteContent } = params || {};
        
        if (!noteContent) {
            return {
                success: false,
                error: 'No note content provided',
            };
        }
        
        const downloadsPath = app.getPath('downloads');
        const timestamp = Date.now();
        const filename = `ciphera-note-${name || 'unknown'}-${timestamp}.json`;
        const filePath = path.join(downloadsPath, filename);
        
        console.log('[Note] Saving to:', filePath);
        
        // Write the note content
        const contentString = typeof noteContent === 'string' 
            ? noteContent 
            : JSON.stringify(noteContent, null, 2);
        fs.writeFileSync(filePath, contentString, 'utf8');
        
        console.log('[Note] Saved successfully');
        
        return {
            success: true,
            path: filePath,
            filename,
        };
    } catch (e) {
        console.error('[Note] Save error:', e);
        return {
            success: false,
            error: e.message,
        };
    }
});

console.log('[Electron] Main process started');
console.log('[Electron] isDev:', isDev);
console.log('[Electron] Working directory:', getWorkingDirectory());
