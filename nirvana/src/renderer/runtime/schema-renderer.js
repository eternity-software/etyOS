import {
  createBadge,
  createButton,
  createCard,
  createGrid,
  createInput,
  createListRow,
  createNotification,
  createPanelShell,
  createSection,
  createSegmentedControl,
  createSlider,
  createStatusMessage,
  createStat,
  createSwitch,
  createTextArea
} from "../ui-kit/components.js";

function applyPresentation(target, node) {
  if (node.className) {
    target.classList.add(...String(node.className).split(/\s+/).filter(Boolean));
  }

  if (node.style) {
    for (const [property, value] of Object.entries(node.style)) {
      target.style[property] = value;
    }
  }

  if (node.vars) {
    for (const [property, value] of Object.entries(node.vars)) {
      const name = property.startsWith("--") ? property : `--${property}`;
      target.style.setProperty(name, value);
    }
  }

  if (node.stackOnMobile) {
    target.dataset.stackOnMobile = "true";
  }

  return target;
}

function bindNodeEvent(target, node, context, domEventName, eventName, readValue) {
  if (!node.id || typeof context?.dispatchEvent !== "function") {
    return;
  }

  target.addEventListener(domEventName, async event => {
    try {
      await context.dispatchEvent({
        nodeId: node.id,
        event: eventName,
        value: readValue ? readValue(event) : null
      });
    } catch (error) {
      console.error("Nirvana callback dispatch failed", error);
    }
  });
}

function appendChildren(node, children = [], context) {
  for (const child of children) {
    node.append(renderSceneNode(child, context));
  }

  return node;
}

function renderPageNode(node, context) {
  const page = document.createElement("main");
  page.className = "nv-page";
  applyPresentation(page, node);
  return appendChildren(page, node.children, context);
}

function renderPanelShellNode(node, context) {
  const panel = createPanelShell({
    eyebrow: node.eyebrow,
    title: node.title,
    subtitle: node.subtitle,
    badge: node.badge
  });

  applyPresentation(panel, node);
  return appendChildren(panel, node.children, context);
}

function renderSectionNode(node, context) {
  const section = createSection({
    title: node.title,
    description: node.description
  });

  applyPresentation(section, node);
  return appendChildren(section, node.children, context);
}

function renderGridNode(node, context) {
  const grid = createGrid("ui-grid");
  applyPresentation(grid, node);
  return appendChildren(grid, node.children, context);
}

function renderListRowNode(node) {
  const row = createListRow({
    title: node.title,
    subtitle: node.subtitle,
    meta: node.meta,
    trailing: node.badge
      ? createBadge({
          label: node.badge.label,
          accent: Boolean(node.badge.accent)
        })
      : undefined
  });

  return applyPresentation(row, node);
}

export function renderSceneNode(node, context = null) {
  switch (node.kind) {
    case "page":
      return renderPageNode(node, context);
    case "panel_shell":
      return renderPanelShellNode(node, context);
    case "section":
      return renderSectionNode(node, context);
    case "grid":
      return renderGridNode(node, context);
    case "stat":
      return applyPresentation(createStat(node), node);
    case "button": {
      const button = applyPresentation(
        createButton({
          ...node,
          kind: node.variant ?? node.kind
        }),
        node
      );
      bindNodeEvent(button, node, context, "click", "click");
      return button;
    }
    case "segmented": {
      const segmented = applyPresentation(
        createSegmentedControl(node.items ?? [], node.activeIndex ?? 0),
        node
      );
      segmented
        .querySelectorAll(".ui-segmented-item")
        .forEach((button, index) => {
          bindNodeEvent(button, node, context, "click", "select", () => ({
            index,
            label: node.items?.[index] ?? ""
          }));
        });
      return segmented;
    }
    case "input": {
      const field = applyPresentation(createInput(node), node);
      const input = field.querySelector(".ui-input");
      bindNodeEvent(input, node, context, "change", "change", () => input.value);
      return field;
    }
    case "textarea": {
      const field = applyPresentation(createTextArea(node), node);
      const textarea = field.querySelector(".ui-textarea");
      bindNodeEvent(textarea, node, context, "change", "change", () => textarea.value);
      return field;
    }
    case "switch": {
      const switchRow = applyPresentation(createSwitch(node), node);
      const input = switchRow.querySelector(".ui-switch-input");
      bindNodeEvent(input, node, context, "change", "change", () => input.checked);
      return switchRow;
    }
    case "slider": {
      const slider = applyPresentation(createSlider(node), node);
      const input = slider.querySelector(".ui-slider");
      bindNodeEvent(input, node, context, "input", "input", () =>
        Number.parseFloat(input.value)
      );
      return slider;
    }
    case "card":
      return applyPresentation(createCard(node), node);
    case "list_row":
      return renderListRowNode(node);
    case "notification":
      return applyPresentation(createNotification(node), node);
    case "status_message":
      return applyPresentation(createStatusMessage(node), node);
    default:
      return createStatusMessage({
        title: "Unknown Schema Node",
        body: `Nirvana received an unsupported node kind: ${node.kind}`,
        tone: "error"
      });
  }
}
