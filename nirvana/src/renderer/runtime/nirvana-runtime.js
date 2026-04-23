import { appletRegistry } from "./applet-registry.js";

export class NirvanaRuntime {
  constructor({ root, host }) {
    this.root = root;
    this.host = host;
  }

  async mount(appletId, options = {}) {
    const applet = appletRegistry.get(appletId);

    if (!applet) {
      throw new Error(`Unknown applet: ${appletId}`);
    }

    this.root.innerHTML = "";
    await applet.render(this.root, {
      host: this.host,
      appletId: applet.id,
      title: applet.title,
      windowRole: options.windowRole ?? null
    });
  }
}
