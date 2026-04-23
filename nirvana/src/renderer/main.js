import { appletRegistry } from "./runtime/applet-registry.js";
import { NirvanaRuntime } from "./runtime/nirvana-runtime.js";
import { componentShowcaseApplet } from "./applets/component-showcase/index.js";

appletRegistry.register(componentShowcaseApplet);

const search = new URLSearchParams(window.location.search);
const appletId = search.get("applet") || "component-showcase";
const windowRole = search.get("role") || null;

const runtime = new NirvanaRuntime({
  root: document.getElementById("app"),
  host: window.nirvanaHost
});

await runtime.mount(appletId, {
  windowRole
});
