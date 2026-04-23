const MAX_STYLE_ENTRIES = 48;
const MAX_CHILDREN = 128;
const MAX_DEPTH = 20;
const MAX_EVENT_VALUE_DEPTH = 8;
const MAX_EVENT_OBJECT_KEYS = 32;
const MAX_EVENT_ARRAY_LENGTH = 64;
const MAX_COMMANDS = 8;
const ALLOWED_KINDS = new Set([
  "page",
  "panel_shell",
  "section",
  "grid",
  "stat",
  "button",
  "segmented",
  "input",
  "slider",
  "switch",
  "textarea",
  "card",
  "list_row",
  "notification",
  "status_message"
]);
const ALLOWED_TONES = new Set(["default", "accent", "muted", "error"]);
const ALLOWED_BUTTON_VARIANTS = new Set([
  "primary",
  "secondary",
  "ghost",
  "danger"
]);
const ALLOWED_EVENTS = new Set(["click", "select", "change", "input"]);
const ALLOWED_COMMANDS = new Set(["open_window"]);
const STYLE_KEY_RE = /^-?-?[a-zA-Z][a-zA-Z0-9-]*$/;
const APPLET_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const WINDOW_ID_RE = /^[0-9]{1,24}$/;
const WINDOW_ROLE_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const SAFE_STYLE_VALUE_RE = /^(?!.*(?:expression\s*\(|javascript:)).*$/i;

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

function validateWindowId(value, label = "windowId") {
  validateString(value, label, 24);
  assert(WINDOW_ID_RE.test(value), `${label} must be a numeric string`);
}

function validateWindowRole(value, label = "windowRole") {
  if (value == null) {
    return;
  }

  validateString(value, label, 64);
  assert(WINDOW_ROLE_RE.test(value), `${label} contains invalid characters`);
}

function validateRequestId(value, label = "request_id") {
  assert(Number.isSafeInteger(value), `${label} must be a safe integer`);
  assert(value > 0, `${label} must be positive`);
}

function validateStyleMap(style, label) {
  if (style === undefined) {
    return;
  }

  assert(style && typeof style === "object" && !Array.isArray(style), `${label} must be an object`);
  const entries = Object.entries(style);
  assert(entries.length <= MAX_STYLE_ENTRIES, `${label} exceeds max entry count`);

  for (const [key, value] of entries) {
    validateString(key, `${label} key`, 64);
    assert(STYLE_KEY_RE.test(key), `${label} contains unsafe key '${key}'`);
    validateString(value, `${label}.${key}`, 512);
    assert(
      SAFE_STYLE_VALUE_RE.test(value),
      `${label}.${key} contains an unsafe CSS value`
    );
  }
}

function validateBadge(badge, label = "badge") {
  if (badge == null) {
    return;
  }

  assert(typeof badge === "object" && !Array.isArray(badge), `${label} must be an object`);
  validateString(badge.label, `${label}.label`, 64);
  assert(typeof badge.accent === "boolean", `${label}.accent must be boolean`);
}

function validateChildren(children, depth) {
  assert(Array.isArray(children), "children must be an array");
  assert(children.length <= MAX_CHILDREN, `children exceeds max length ${MAX_CHILDREN}`);
  for (const child of children) {
    validateSceneNode(child, depth + 1);
  }
}

function validateSceneNode(node, depth = 0) {
  assert(depth <= MAX_DEPTH, `scene exceeds max depth ${MAX_DEPTH}`);
  assert(node && typeof node === "object" && !Array.isArray(node), "scene node must be an object");
  validateString(node.kind, "scene.kind", 32);
  assert(ALLOWED_KINDS.has(node.kind), `unsupported scene node kind '${node.kind}'`);

  if (node.id !== undefined) {
    validateString(node.id, `${node.kind}.id`, 128);
  }

  if (node.className !== undefined) {
    validateString(node.className, `${node.kind}.className`, 128);
  }

  validateStyleMap(node.style, `${node.kind}.style`);
  validateStyleMap(node.vars, `${node.kind}.vars`);

  switch (node.kind) {
    case "page":
    case "panel_shell":
    case "section":
    case "grid":
      validateChildren(node.children, depth);
      break;
    case "stat":
      validateString(node.label, "stat.label", 64);
      validateString(node.value, "stat.value", 256);
      break;
    case "button":
      validateString(node.label, "button.label", 64);
      validateString(node.variant, "button.variant", 32);
      assert(ALLOWED_BUTTON_VARIANTS.has(node.variant), `unsupported button variant '${node.variant}'`);
      assert(typeof node.quiet === "boolean", "button.quiet must be boolean");
      break;
    case "segmented":
      assert(Array.isArray(node.items), "segmented.items must be an array");
      assert(node.items.length > 0 && node.items.length <= 12, "segmented.items count out of range");
      node.items.forEach((item, index) => validateString(item, `segmented.items[${index}]`, 64));
      assert(Number.isInteger(node.activeIndex), "segmented.activeIndex must be integer");
      assert(
        node.activeIndex >= 0 && node.activeIndex < node.items.length,
        "segmented.activeIndex is out of range"
      );
      break;
    case "input":
    case "textarea":
      validateString(node.label, `${node.kind}.label`, 64);
      validateString(node.placeholder, `${node.kind}.placeholder`, 256);
      validateString(node.value, `${node.kind}.value`, 4096);
      if (node.leading !== undefined && node.leading !== null) {
        validateString(node.leading, `${node.kind}.leading`, 32);
      }
      break;
    case "slider":
      validateString(node.label, "slider.label", 64);
      assert(Number.isFinite(node.value), "slider.value must be numeric");
      assert(Number.isFinite(node.min), "slider.min must be numeric");
      assert(Number.isFinite(node.max), "slider.max must be numeric");
      assert(node.min <= node.max, "slider.min must be <= slider.max");
      assert(node.value >= node.min && node.value <= node.max, "slider.value must be within range");
      break;
    case "switch":
      validateString(node.label, "switch.label", 64);
      validateString(node.hint, "switch.hint", 256);
      assert(typeof node.checked === "boolean", "switch.checked must be boolean");
      break;
    case "card":
      validateString(node.title, "card.title", 64);
      validateString(node.description, "card.description", 512);
      validateString(node.tone, "card.tone", 32);
      assert(ALLOWED_TONES.has(node.tone), `unsupported card tone '${node.tone}'`);
      break;
    case "list_row":
      validateString(node.title, "list_row.title", 64);
      validateString(node.subtitle, "list_row.subtitle", 256);
      validateString(node.meta, "list_row.meta", 128);
      validateBadge(node.badge, "list_row.badge");
      break;
    case "notification":
      validateString(node.title, "notification.title", 64);
      validateString(node.body, "notification.body", 512);
      validateString(node.badge, "notification.badge", 64);
      validateString(node.tone, "notification.tone", 32);
      assert(ALLOWED_TONES.has(node.tone), `unsupported notification tone '${node.tone}'`);
      break;
    case "status_message":
      validateString(node.title, "status_message.title", 64);
      validateString(node.body, "status_message.body", 512);
      validateString(node.tone, "status_message.tone", 32);
      assert(ALLOWED_TONES.has(node.tone), `unsupported status_message tone '${node.tone}'`);
      break;
  }
}

function validateSceneEnvelope(payload) {
  assert(payload && typeof payload === "object" && !Array.isArray(payload), "scene payload must be an object");
  validateSceneNode(payload.scene);
  validateHostCommands(payload.commands);
}

function validateEventValue(value, depth = 0, label = "event.value") {
  assert(depth <= MAX_EVENT_VALUE_DEPTH, `${label} exceeds max depth ${MAX_EVENT_VALUE_DEPTH}`);

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
    assert(value.length <= MAX_EVENT_ARRAY_LENGTH, `${label} exceeds max length ${MAX_EVENT_ARRAY_LENGTH}`);
    value.forEach((entry, index) => validateEventValue(entry, depth + 1, `${label}[${index}]`));
    return;
  }

  assert(typeof value === "object", `${label} must be a JSON-compatible value`);
  const entries = Object.entries(value);
  assert(entries.length <= MAX_EVENT_OBJECT_KEYS, `${label} exceeds max object size`);

  for (const [key, nestedValue] of entries) {
    validateString(key, `${label} key`, 64);
    validateEventValue(nestedValue, depth + 1, `${label}.${key}`);
  }
}

function validateEventPayload(payload) {
  assert(payload && typeof payload === "object" && !Array.isArray(payload), "event payload must be an object");
  validateAppletId(payload.appletId, "event.appletId");
  validateString(payload.nodeId, "event.nodeId", 128);
  validateString(payload.event, "event.event", 32);
  assert(ALLOWED_EVENTS.has(payload.event), `unsupported event '${payload.event}'`);
  validateEventValue(payload.value);
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

function validateHostCommands(commands) {
  if (commands === undefined) {
    return;
  }

  assert(Array.isArray(commands), "commands must be an array");
  assert(commands.length <= MAX_COMMANDS, `commands exceeds max length ${MAX_COMMANDS}`);

  for (const command of commands) {
    assert(command && typeof command === "object" && !Array.isArray(command), "command must be an object");
    validateString(command.type, "command.type", 32);
    assert(ALLOWED_COMMANDS.has(command.type), `unsupported command '${command.type}'`);

    if (command.type === "open_window") {
      validateWindowRole(command.role, "command.role");

      if (command.title !== undefined && command.title !== null) {
        validateString(command.title, "command.title", 128);
      }

      if (command.width !== undefined && command.width !== null) {
        assert(Number.isInteger(command.width), "command.width must be an integer");
        assert(command.width >= 240 && command.width <= 2400, "command.width is out of range");
      }

      if (command.height !== undefined && command.height !== null) {
        assert(Number.isInteger(command.height), "command.height must be an integer");
        assert(command.height >= 180 && command.height <= 1800, "command.height is out of range");
      }

      if (command.resizable !== undefined && command.resizable !== null) {
        assert(typeof command.resizable === "boolean", "command.resizable must be boolean");
      }
    }
  }
}

module.exports = {
  validateAppletId,
  validateEventPayload,
  validateHostCommands,
  validateLoadAppletOptions,
  validateRequestId,
  validateSceneEnvelope,
  validateWindowId,
  validateWindowRole
};
