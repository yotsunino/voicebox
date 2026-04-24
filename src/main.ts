import { app, BrowserWindow, ipcMain, shell } from 'electron';
import { join } from 'path';

// Keep a global reference to prevent garbage collection
let mainWindow: BrowserWindow | null = null;

/**
 * Creates the main application window with appropriate settings
 * for a voice/audio-focused desktop application.
 */
function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 900,
    height: 600,
    minWidth: 700,
    minHeight: 500,
    titleBarStyle: 'hiddenInset',
    backgroundColor: '#0f0f0f',
    show: false,
    webPreferences: {
      preload: join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
    icon: join(__dirname, '../assets/icon.png'),
  });

  // Load the renderer — in dev mode use Vite dev server, otherwise load built files
  if (process.env.NODE_ENV === 'development') {
    mainWindow.loadURL('http://localhost:5173');
    mainWindow.webContents.openDevTools();
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'));
  }

  // Show window once ready to avoid flash of blank screen
  mainWindow.once('ready-to-show', () => {
    mainWindow?.show();
  });

  // Open external links in the system default browser
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: 'deny' };
  });

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

/**
 * Register IPC handlers for renderer <-> main process communication.
 */
function registerIpcHandlers(): void {
  // Return the current app version
  ipcMain.handle('app:get-version', () => app.getVersion());

  // Open a URL in the default browser
  ipcMain.handle('app:open-external', (_event, url: string) => {
    shell.openExternal(url);
  });

  // Quit the application
  ipcMain.on('app:quit', () => {
    app.quit();
  });
}

// ── App lifecycle ────────────────────────────────────────────────────────────

app.whenReady().then(() => {
  registerIpcHandlers();
  createWindow();

  // On macOS re-create the window when the dock icon is clicked and no windows are open
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

// Quit when all windows are closed, except on macOS
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
