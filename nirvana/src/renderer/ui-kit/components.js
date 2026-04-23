function element(tag, className, text) {
  const node = document.createElement(tag);

  if (className) {
    node.className = className;
  }

  if (text !== undefined) {
    node.textContent = text;
  }

  return node;
}

export function createPanelShell({ eyebrow, title, subtitle, badge }) {
  const shell = element("section", "ui-panel");
  const hero = element("header", "ui-hero");
  const copy = element("div", "ui-hero-copy");

  if (eyebrow) {
    copy.append(element("div", "ui-eyebrow", eyebrow));
  }

  copy.append(element("h1", "ui-title", title));
  copy.append(element("p", "ui-subtitle", subtitle));
  hero.append(copy);

  if (badge) {
    hero.append(createBadge({ label: badge, accent: true }));
  }

  shell.append(hero);
  return shell;
}

export function createSection({ title, description }) {
  const section = element("section", "ui-section");
  const header = element("div", "ui-section-header");

  header.append(element("h2", "ui-section-title", title));

  if (description) {
    header.append(element("p", "ui-section-description", description));
  }

  section.append(header);
  return section;
}

export function createGrid(className = "ui-grid") {
  return element("div", className);
}

export function createCard({ title, description, tone = "default" }) {
  const card = element("article", `ui-card ui-card-${tone}`);
  card.append(element("h3", "ui-card-title", title));

  if (description) {
    card.append(element("p", "ui-card-description", description));
  }

  return card;
}

export function createButton({ label, kind = "primary", quiet = false }) {
  const button = element(
    "button",
    `ui-button ui-button-${kind}${quiet ? " ui-button-quiet" : ""}`,
    label
  );

  button.type = "button";
  return button;
}

export function createInput({
  label,
  placeholder = "",
  value = "",
  leading = null
}) {
  const field = element("label", "ui-field");
  const labelNode = element("span", "ui-field-label", label);
  const control = element("div", "ui-input-shell");
  const input = document.createElement("input");

  input.className = "ui-input";
  input.placeholder = placeholder;
  input.value = value;

  if (leading) {
    control.append(element("span", "ui-input-leading", leading));
  }

  control.append(input);
  field.append(labelNode, control);

  return field;
}

export function createTextArea({ label, placeholder = "", value = "" }) {
  const field = element("label", "ui-field");
  const labelNode = element("span", "ui-field-label", label);
  const area = document.createElement("textarea");

  area.className = "ui-textarea";
  area.placeholder = placeholder;
  area.value = value;

  field.append(labelNode, area);
  return field;
}

export function createSwitch({ label, hint, checked = false }) {
  const row = element("label", "ui-switch-row");
  const copy = element("div", "ui-switch-copy");
  const title = element("span", "ui-switch-label", label);
  const note = element("span", "ui-switch-hint", hint);
  const input = document.createElement("input");
  const control = element("span", "ui-switch");

  input.type = "checkbox";
  input.className = "ui-switch-input";
  input.checked = checked;

  copy.append(title, note);
  row.append(copy, input, control);
  return row;
}

export function createSlider({ label, value = 50, min = 0, max = 100 }) {
  const wrap = element("label", "ui-field");
  const header = element("div", "ui-slider-header");
  const title = element("span", "ui-field-label", label);
  const current = element("span", "ui-slider-value", String(value));
  const input = document.createElement("input");

  input.type = "range";
  input.className = "ui-slider";
  input.min = String(min);
  input.max = String(max);
  input.value = String(value);

  input.addEventListener("input", () => {
    current.textContent = input.value;
  });

  header.append(title, current);
  wrap.append(header, input);
  return wrap;
}

export function createSegmentedControl(items, activeIndex = 0) {
  const group = element("div", "ui-segmented");

  items.forEach((item, index) => {
    const button = createButton({
      label: item,
      kind: index === activeIndex ? "secondary" : "ghost",
      quiet: true
    });

    button.classList.add("ui-segmented-item");
    group.append(button);
  });

  return group;
}

export function createBadge({ label, accent = false }) {
  return element("span", `ui-badge${accent ? " ui-badge-accent" : ""}`, label);
}

export function createStat({ label, value }) {
  const stat = element("div", "ui-stat");
  stat.append(element("span", "ui-stat-value", value));
  stat.append(element("span", "ui-stat-label", label));
  return stat;
}

export function createListRow({ title, subtitle, meta, trailing }) {
  const row = element("div", "ui-list-row");
  const copy = element("div", "ui-list-copy");
  const heading = element("span", "ui-list-title", title);

  copy.append(heading);

  if (subtitle) {
    copy.append(element("span", "ui-list-subtitle", subtitle));
  }

  row.append(copy);

  if (meta) {
    row.append(element("span", "ui-list-meta", meta));
  }

  if (trailing) {
    row.append(trailing);
  }

  return row;
}

export function createNotification({
  title,
  body,
  badge = "New",
  tone = "default"
}) {
  const card = element("article", `ui-notification ui-notification-${tone}`);
  const top = element("div", "ui-notification-top");

  top.append(element("strong", "ui-notification-title", title));
  top.append(createBadge({ label: badge, accent: tone === "accent" }));
  card.append(top);
  card.append(element("p", "ui-notification-body", body));
  return card;
}

export function createStatusMessage({ title, body, tone = "default" }) {
  const card = element("section", `ui-status-message ui-status-message-${tone}`);
  card.append(element("h2", "ui-status-title", title));

  if (body) {
    card.append(element("p", "ui-status-body", body));
  }

  return card;
}
