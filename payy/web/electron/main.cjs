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
            reject({
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
                reject({
                    success: false,
                    error: stderr || stdout || `Process exited with code ${code}`,
                    stdout,
                    stderr,
                    code,
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
        const cwd = getWorkingDirectory();
        const walletPath = path.join(cwd, `${name}.json`);
        
        if (!fs.existsSync(walletPath)) {
            return { exists: false };
        }
        
        const content = fs.readFileSync(walletPath, 'utf8');
        const wallet = JSON.parse(content);
        
        return {
            exists: true,
            wallet,
        };
    } catch (e) {
        return {
            exists: false,
            error: e.message,
        };
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
