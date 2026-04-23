const { contextBridge, ipcRenderer } = require("electron");

const APPLET_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const WINDOW_ROLE_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const ALLOWED_EVENTS = new Set(["click", "select", "change", "input"]);

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function validateString(value, label, maxLength = 4096) {
  assert(typeof value === "string", `${label} must be a string`);
  assert(value.length <= maxLength, `${label} exceeds max length ${maxLength}`);
}

function validateAppletId(value, label = "appletId") {
  validateString(value, label, 64);
  assert(APPLET_ID_RE.test(value), `${label} contains invalid characters`);
}

function validateWindowRole(value, label = "windowRole") {
  if (value == null) {
    return;
  }

  validateString(value, label, 64);
  assert(WINDOW_ROLE_RE.test(value), `${label} contains invalid characters`);
}

function validateLoadAppletOptions(options) {
  if (options === undefined) {
    return;
  }

  assert(
    options && typeof options === "object" && !Array.isArray(options),
    "load options must be an object"
  );
  validateWindowRole(options.role, "load.options.role");
}

function validateEventValue(value, depth = 0, label = "event.value") {
  assert(depth <= 8, `${label} exceeds max depth 8`);

  if (value == null) {
    return;
  }

  if (typeof value === "string") {
    validateString(value, label, 4096);
    return;
  }

  if (typeof value === "number") {
    assert(Number.isFinite(value), `${label} must be finite`);
    return;
  }

  if (typeof value === "boolean") {
    return;
  }

  if (Array.isArray(value)) {
    assert(value.length <= 64, `${label} exceeds max length 64`);
    value.forEach((entry, index) =>
      validateEventValue(entry, depth + 1, `${label}[${index}]`)
    );
    return;
  }

  assert(typeof value === "object", `${label} must be a JSON-compatible value`);
  const entries = Object.entries(value);
  assert(entries.length <= 32, `${label} exceeds max object size`);

  for (const [key, nestedValue] of entries) {
    validateString(key, `${label} key`, 64);
    validateEventValue(nestedValue, depth + 1, `${label}.${key}`);
  }
}

function validateEventPayload(payload) {
  assert(
    payload && typeof payload === "object" && !Array.isArray(payload),
    "event payload must be an object"
  );
  validateAppletId(payload.appletId, "event.appletId");
  validateString(payload.nodeId, "event.nodeId", 128);
  validateString(payload.event, "event.event", 32);
  assert(ALLOWED_EVENTS.has(payload.event), `unsupported event '${payload.event}'`);
  validateEventValue(payload.value);
}

const hostApi = Object.freeze({
  platform: process.platform,
  versions: Object.freeze({
    chrome: process.versions.chrome,
    electron: process.versions.electron,
    node: process.versions.node
  }),
  loadRustApplet(appletId, options = undefined) {
    validateAppletId(appletId);
    validateLoadAppletOptions(options);
    return ipcRenderer.invoke("nirvana:load-rust-applet", {
      appletId,
      options
    });
  },
  dispatchRustAppletEvent(appletId, payload) {
    validateAppletId(appletId);
    const request = {
      ...payload,
      appletId
    };

    validateEventPayload(request);
    return ipcRenderer.invoke("nirvana:dispatch-rust-applet-event", request);
  },
  onRustAppletScene(listener) {
    assert(typeof listener === "function", "scene listener must be a function");

    const wrapped = (_event, payload) => {
      listener(payload);
    };

    ipcRenderer.on("nirvana:rust-applet-scene", wrapped);
    return () => {
      ipcRenderer.removeListener("nirvana:rust-applet-scene", wrapped);
    };
  },
  onRustAppletError(listener) {
    assert(typeof listener === "function", "error listener must be a function");

    const wrapped = (_event, payload) => {
      listener(payload);
    };

    ipcRenderer.on("nirvana:rust-applet-error", wrapped);
    return () => {
      ipcRenderer.removeListener("nirvana:rust-applet-error", wrapped);
    };
  }
});

contextBridge.exposeInMainWorld("nirvanaHost", hostApi);
