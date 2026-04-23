const { app, BrowserWindow, ipcMain, Menu } = require("electron");
const path = require("path");
const {
  validateAppletId,
  validateEventPayload,
  validateHostCommands,
  validateLoadAppletOptions
} = require("./protocol-validation");
const {
  dispatchRustAppletEvent,
  getAppletDefinition,
  handleWindowClosed,
  listAppletWindowIds,
  loadRustAppletModel,
  shutdownRustApplets
} = require("./rust-applets");

if (process.platform === "linux") {
  app.setName("nirvana");
  app.commandLine.appendSwitch("class", "nirvana");
  app.commandLine.appendSwitch("ozone-platform", "wayland");
  app.commandLine.appendSwitch("enable-smooth-scrolling");
}

if (process.env.NIRVANA_DISABLE_GPU === "1") {
  app.disableHardwareAcceleration();
}

function hardenWebContents(contents) {
  contents.setWindowOpenHandler(() => ({ action: "deny" }));
  contents.on("will-navigate", event => {
    event.preventDefault();
  });
  contents.on("will-attach-webview", event => {
    event.preventDefault();
  });
  contents.session.setPermissionRequestHandler((_webContents, _permission, callback) => {
    callback(false);
  });
}

function createMainWindow(options = {}) {
  const {
    title = "Nirvana",
    width = 1560,
    height = 980,
    minWidth = 1120,
    minHeight = 760,
    rendererAppletId = "component-showcase",
    windowRole = null,
    resizable = true
  } = options;
  const window = new BrowserWindow({
    width,
    height,
    minWidth,
    minHeight,
    resizable,
    show: false,
    autoHideMenuBar: true,
    backgroundColor: "#eef6ff",
    title,
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 18, y: 18 },
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
      allowRunningInsecureContent: false,
      webviewTag: false,
      spellcheck: false,
      devTools: !app.isPackaged
    }
  });

  hardenWebContents(window.webContents);
  window.setMenuBarVisibility(false);

  window.once("ready-to-show", () => {
    window.show();
  });

  window.on("closed", () => {
    const affectedApplets = handleWindowClosed(window.id);

    for (const appletId of affectedApplets) {
      refreshAppletWindows(appletId).catch(() => {});
    }
  });

  window.loadFile(path.join(__dirname, "../renderer/index.html"), {
    query: {
      applet: rendererAppletId,
      role: windowRole ?? ""
    }
  });

  return window;
}

function executeHostCommands(appletId, commands) {
  validateHostCommands(commands);

  if (!commands?.length) {
    return;
  }

  const applet = getAppletDefinition(appletId);

  if (!applet) {
    throw new Error(`Unknown Rust applet: ${appletId}`);
  }

  for (const command of commands) {
    if (command.type !== "open_window") {
      continue;
    }

    const width = command.width ?? 520;
    const height = command.height ?? 320;

    createMainWindow({
      title: command.title ?? "Nirvana",
      width,
      height,
      minWidth: width,
      minHeight: height,
      rendererAppletId: applet.rendererAppletId,
      windowRole: command.role ?? null,
      resizable: command.resizable ?? true
    });
  }
}

async function refreshAppletWindows(appletId, excludeWindowId = null) {
  const windowIds = listAppletWindowIds(appletId);

  for (const windowId of windowIds) {
    if (excludeWindowId !== null && String(windowId) === String(excludeWindowId)) {
      continue;
    }

    const browserWindow = BrowserWindow.fromId(Number(windowId));

    if (!browserWindow || browserWindow.isDestroyed()) {
      continue;
    }

    try {
      const response = await loadRustAppletModel(appletId, windowId);
      browserWindow.webContents.send("nirvana:rust-applet-scene", {
        appletId,
        scene: response.scene
      });
    } catch (error) {
      browserWindow.webContents.send("nirvana:rust-applet-error", {
        appletId,
        message: error?.message ?? String(error)
      });
    }
  }
}

ipcMain.handle("nirvana:load-rust-applet", async (event, payload) => {
  validateAppletId(payload.appletId);
  validateLoadAppletOptions(payload.options);
  const window = BrowserWindow.fromWebContents(event.sender);

  if (!window) {
    throw new Error("Unable to resolve BrowserWindow for Rust applet mount.");
  }

  const response = await loadRustAppletModel(payload.appletId, window.id, payload.options);
  executeHostCommands(payload.appletId, response.commands);
  await refreshAppletWindows(payload.appletId, window.id);
  return response;
});

ipcMain.handle("nirvana:dispatch-rust-applet-event", async (event, payload) => {
  validateEventPayload(payload);
  const window = BrowserWindow.fromWebContents(event.sender);

  if (!window) {
    throw new Error("Unable to resolve BrowserWindow for Rust applet event.");
  }

  const response = await dispatchRustAppletEvent(payload.appletId, window.id, payload);
  executeHostCommands(payload.appletId, response.commands);
  return response;
});

app.whenReady().then(() => {
  Menu.setApplicationMenu(null);

  app.on("web-contents-created", (_event, contents) => {
    hardenWebContents(contents);
  });

  createMainWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createMainWindow();
    }
  });
});

app.on("window-all-closed", () => {
  shutdownRustApplets();

  if (process.platform !== "darwin") {
    app.quit();
  }
});
