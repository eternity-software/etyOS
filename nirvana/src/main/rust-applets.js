const { spawn } = require("node:child_process");
const { existsSync, readFileSync } = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");
const {
  validateAppletId,
  validateEventPayload,
  validateLoadAppletOptions,
  validateRequestId,
  validateSceneEnvelope,
  validateWindowId
} = require("./protocol-validation");

const REQUEST_TIMEOUT_MS = 5000;
const SHUTDOWN_GRACE_MS = 1200;
const KILL_GRACE_MS = 2500;
const MAX_STDERR_LENGTH = 16000;
const MAX_LINE_LENGTH = 512000;
const rootDir = path.join(__dirname, "../../..");
const nirvanaDir = path.join(__dirname, "../..");
const registryPath = path.join(nirvanaDir, "applets/registry.json");
const APPLET_MODE = process.env.NIRVANA_APPLET_MODE ?? "auto";
const sessions = new Map();
const windowToApplets = new Map();

function createAppletEnv() {
  const allowedKeys = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "SHELL",
    "TERM",
    "TMPDIR",
    "TEMP",
    "TMP",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "CARGO_HOME",
    "RUSTUP_HOME"
  ];
  const env = {};

  for (const key of allowedKeys) {
    if (process.env[key]) {
      env[key] = process.env[key];
    }
  }

  env.PATH = env.PATH ?? process.env.PATH ?? "";
  env.XDG_SESSION_TYPE = "wayland";

  return env;
}

function normalizeAppletDefinition(entry) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new Error("Applet registry entries must be objects.");
  }

  validateAppletId(entry.id, "applet registry id");

  if (typeof entry.manifestPath !== "string" || entry.manifestPath.length === 0) {
    throw new Error(`Applet ${entry.id} is missing manifestPath.`);
  }

  if (typeof entry.packageName !== "string" || entry.packageName.length === 0) {
    throw new Error(`Applet ${entry.id} is missing packageName.`);
  }
  if (typeof entry.rendererAppletId !== "string" || entry.rendererAppletId.length === 0) {
    throw new Error(`Applet ${entry.id} is missing rendererAppletId.`);
  }

  const manifestPath = path.resolve(nirvanaDir, entry.manifestPath);

  if (!existsSync(manifestPath)) {
    throw new Error(`Applet ${entry.id} manifest does not exist: ${manifestPath}`);
  }

  return {
    id: entry.id,
    manifestPath,
    packageName: entry.packageName,
    rendererAppletId: entry.rendererAppletId
  };
}

function loadAppletRegistry() {
  const raw = readFileSync(registryPath, "utf8");
  const parsed = JSON.parse(raw);
  const entries = parsed?.applets;

  if (!Array.isArray(entries) || entries.length === 0) {
    throw new Error("Nirvana applet registry must contain a non-empty 'applets' array.");
  }

  const registry = new Map();

  for (const entry of entries) {
    const applet = normalizeAppletDefinition(entry);

    if (registry.has(applet.id)) {
      throw new Error(`Duplicate Rust applet id in registry: ${applet.id}`);
    }

    registry.set(applet.id, applet);
  }

  return registry;
}

function resolveSpawnTarget(applet) {
  const manifestDir = path.dirname(applet.manifestPath);
  const binaryName =
    process.platform === "win32" ? `${applet.packageName}.exe` : applet.packageName;
  const releaseBinaryPath = path.join(manifestDir, "target/release", binaryName);
  const releaseBinaryExists = existsSync(releaseBinaryPath);

  if (APPLET_MODE === "release") {
    if (!releaseBinaryExists) {
      throw new Error(
        `Rust applet '${applet.id}' is configured for release mode but '${releaseBinaryPath}' does not exist.`
      );
    }

    return {
      command: releaseBinaryPath,
      args: [],
      cwd: manifestDir
    };
  }

  if (APPLET_MODE === "auto" && releaseBinaryExists) {
    return {
      command: releaseBinaryPath,
      args: [],
      cwd: manifestDir
    };
  }

  return {
    command: "cargo",
    args: ["run", "--quiet", "--manifest-path", applet.manifestPath],
    cwd: rootDir
  };
}

function rejectPendingRequests(pending, error) {
  for (const { reject, timeoutId } of pending.values()) {
    clearTimeout(timeoutId);
    reject(error);
  }

  pending.clear();
}

const applets = loadAppletRegistry();

class RustAppletSession {
  constructor(applet) {
    const spawnTarget = resolveSpawnTarget(applet);

    this.appletId = applet.id;
    this.manifestPath = applet.manifestPath;
    this.packageName = applet.packageName;
    this.pending = new Map();
    this.openWindows = new Set();
    this.windowRoles = new Map();
    this.nextRequestId = 1;
    this.closeStarted = false;
    this.closeTimer = null;
    this.killTimer = null;
    this.lastError = null;

    this.child = spawn(
      spawnTarget.command,
      spawnTarget.args,
      {
        cwd: spawnTarget.cwd,
        env: createAppletEnv(),
        stdio: ["pipe", "pipe", "pipe"]
      }
    );

    this.stderr = "";
    this.readline = readline.createInterface({
      input: this.child.stdout
    });

    this.readline.on("line", line => {
      this.handleLine(line);
    });

    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", chunk => {
      this.stderr = `${this.stderr}${chunk}`.slice(-MAX_STDERR_LENGTH);
    });

    this.closed = new Promise(resolve => {
      this.child.on("error", error => {
        this.lastError = error;
      });
      this.child.on("close", (code, signal) => {
        this.finalizeClose(code, signal);
        resolve();
      });
    });
  }

  finalizeClose(code, signal) {
    if (this.closeStarted) {
      return;
    }

    this.closeStarted = true;
    clearTimeout(this.closeTimer);
    clearTimeout(this.killTimer);
    this.readline.close();

    const exitError =
      this.lastError ||
      (code === 0
        ? null
        : new Error(
            this.stderr.trim() ||
              `Rust applet ${this.appletId} exited with status ${code}${signal ? ` (${signal})` : ""}`
          ));

    rejectPendingRequests(
      this.pending,
      exitError ??
        new Error(`Rust applet ${this.appletId} closed before replying.`)
    );
  }

  failSession(error) {
    rejectPendingRequests(this.pending, error);

    if (this.child.exitCode === null && !this.child.killed) {
      this.lastError = error;
      this.child.kill("SIGTERM");
      this.killTimer = setTimeout(() => {
        if (this.child.exitCode === null && !this.child.killed) {
          this.child.kill("SIGKILL");
        }
      }, KILL_GRACE_MS);
    }
  }

  handleLine(line) {
    if (line.length > MAX_LINE_LENGTH) {
      this.failSession(
        new Error(`Rust applet ${this.appletId} exceeded max stdout line length.`)
      );
      return;
    }

    let message;

    try {
      message = JSON.parse(line);
    } catch (error) {
      this.failSession(
        new Error(`Rust applet ${this.appletId} emitted invalid JSON: ${error.message}`)
      );
      return;
    }

    try {
      if (message.type === "error") {
        if (message.request_id !== null && message.request_id !== undefined) {
          validateRequestId(message.request_id, "error.request_id");
        }

        const pending = this.pending.get(message.request_id);

        if (pending) {
          this.pending.delete(message.request_id);
          clearTimeout(pending.timeoutId);
          pending.reject(
            new Error(message.message || `Rust applet ${this.appletId} returned an error.`)
          );
          return;
        }

        return;
      }

      if (message.type !== "scene") {
        this.failSession(
          new Error(`Rust applet ${this.appletId} emitted unsupported response type '${message.type}'.`)
        );
        return;
      }

      validateRequestId(message.request_id, "scene.request_id");
      const pending = this.pending.get(message.request_id);

      if (!pending) {
        return;
      }

      validateSceneEnvelope({ scene: message.scene });
      this.pending.delete(message.request_id);
      clearTimeout(pending.timeoutId);
      pending.resolve({
        scene: message.scene,
        commands: message.commands ?? []
      });
    } catch (error) {
      this.failSession(error);
    }
  }

  send(message) {
    if (!this.child.stdin.writable) {
      throw new Error(`Rust applet ${this.appletId} stdin is not writable.`);
    }

    const line = `${JSON.stringify(message)}\n`;

    if (line.length > MAX_LINE_LENGTH) {
      throw new Error(`Request to Rust applet ${this.appletId} exceeded max line length.`);
    }

    this.child.stdin.write(line);
  }

  ensureWindow(windowId, options = undefined) {
    const key = String(windowId);
    validateWindowId(key);

    if (this.openWindows.has(key)) {
      return;
    }

    this.openWindows.add(key);
    this.windowRoles.set(key, options?.role ?? null);
    this.send({
      type: "window_opened",
      window_id: key,
      role: options?.role ?? null
    });
  }

  requestScene(windowId, options = undefined) {
    this.ensureWindow(windowId, options);

    const requestId = this.nextRequestId++;

    return new Promise((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        this.pending.delete(requestId);
        reject(
          new Error(
            `Rust applet ${this.appletId} timed out handling render request ${requestId}.`
          )
        );
      }, REQUEST_TIMEOUT_MS);

      this.pending.set(requestId, { resolve, reject, timeoutId });
      this.send({
        type: "render",
        request_id: requestId,
        window_id: String(windowId)
      });
    });
  }

  dispatchEvent(windowId, payload) {
    this.ensureWindow(windowId);
    validateEventPayload(payload);

    const requestId = this.nextRequestId++;

    return new Promise((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        this.pending.delete(requestId);
        reject(
          new Error(
            `Rust applet ${this.appletId} timed out handling event request ${requestId}.`
          )
        );
      }, REQUEST_TIMEOUT_MS);

      this.pending.set(requestId, { resolve, reject, timeoutId });
      this.send({
        type: "event",
        request_id: requestId,
        window_id: String(windowId),
        node_id: payload.nodeId,
        event: payload.event,
        value: payload.value ?? null
      });
    });
  }

  notifyWindowClosed(windowId) {
    const key = String(windowId);

    if (!this.openWindows.delete(key)) {
      return;
    }
    this.windowRoles.delete(key);

    this.send({
      type: "window_closed",
      window_id: key
    });

    if (this.openWindows.size === 0) {
      clearTimeout(this.closeTimer);
      this.closeTimer = setTimeout(() => {
        if (this.child.exitCode === null && !this.child.killed) {
          this.child.kill("SIGTERM");
          this.killTimer = setTimeout(() => {
            if (this.child.exitCode === null && !this.child.killed) {
              this.child.kill("SIGKILL");
            }
          }, KILL_GRACE_MS);
        }
      }, SHUTDOWN_GRACE_MS);
    }
  }

  shutdown() {
    clearTimeout(this.closeTimer);
    clearTimeout(this.killTimer);

    if (this.child.stdin.writable) {
      this.child.stdin.end();
    }

    if (this.child.exitCode === null && !this.child.killed) {
      this.child.kill("SIGTERM");
      this.killTimer = setTimeout(() => {
        if (this.child.exitCode === null && !this.child.killed) {
          this.child.kill("SIGKILL");
        }
      }, KILL_GRACE_MS);
    }
  }
}

function getSession(appletId) {
  const applet = applets.get(appletId);

  if (!applet) {
    throw new Error(`Unknown Rust applet: ${appletId}`);
  }

  let session = sessions.get(appletId);

  if (!session) {
    session = new RustAppletSession(applet);
    sessions.set(appletId, session);

    session.closed.finally(() => {
      sessions.delete(appletId);
    });
  }

  return session;
}

function getAppletDefinition(appletId) {
  return applets.get(appletId) ?? null;
}

function listAppletWindowIds(appletId) {
  const session = sessions.get(appletId);

  if (!session) {
    return [];
  }

  return Array.from(session.openWindows);
}

function rememberWindowApplet(windowId, appletId) {
  const key = String(windowId);
  validateWindowId(key);
  const mounted = windowToApplets.get(key) ?? new Set();

  mounted.add(appletId);
  windowToApplets.set(key, mounted);
}

async function loadRustAppletModel(appletId, windowId, options = undefined) {
  if (!applets.has(appletId)) {
    throw new Error(`Unknown Rust applet: ${appletId}`);
  }

  validateLoadAppletOptions(options);
  rememberWindowApplet(windowId, appletId);
  const session = getSession(appletId);
  return session.requestScene(windowId, options);
}

async function dispatchRustAppletEvent(appletId, windowId, payload) {
  const session = sessions.get(appletId);

  if (!session) {
    throw new Error(`Rust applet is not running: ${appletId}`);
  }

  return session.dispatchEvent(windowId, payload);
}

function handleWindowClosed(windowId) {
  const key = String(windowId);
  const mounted = windowToApplets.get(key);

  if (!mounted) {
    return [];
  }

  for (const appletId of mounted) {
    const session = sessions.get(appletId);

    if (session) {
      session.notifyWindowClosed(windowId);
    }
  }

  windowToApplets.delete(key);
  return Array.from(mounted);
}

function shutdownRustApplets() {
  for (const session of sessions.values()) {
    session.shutdown();
  }

  sessions.clear();
  windowToApplets.clear();
}

module.exports = {
  dispatchRustAppletEvent,
  getAppletDefinition,
  handleWindowClosed,
  listAppletWindowIds,
  loadRustAppletModel,
  shutdownRustApplets
};
