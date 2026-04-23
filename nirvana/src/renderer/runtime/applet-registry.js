class AppletRegistry {
  constructor() {
    this.applets = new Map();
  }

  register(applet) {
    if (!applet || !applet.id || typeof applet.render !== "function") {
      throw new Error("Applet must define an id and render(root, context).");
    }

    this.applets.set(applet.id, applet);
  }

  get(id) {
    return this.applets.get(id);
  }
}

export const appletRegistry = new AppletRegistry();
