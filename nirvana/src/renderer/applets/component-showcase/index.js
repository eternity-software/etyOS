import { renderSceneNode } from "../../runtime/schema-renderer.js";
import { createStatusMessage } from "../../ui-kit/components.js";

export const componentShowcaseApplet = {
  id: "component-showcase",
  title: "Component Showcase",
  async render(root, context) {
    const rustAppletId = "uikit-showcase";
    let unsubscribeScene = null;
    let unsubscribeError = null;
    const loading = createStatusMessage({
      title: "Loading Rust applet",
      body: "Nirvana is requesting the showcase view model from the Rust applet process."
    });

    root.append(loading);

    try {
      const renderScene = scene => {
        root.innerHTML = "";
        root.append(
          renderSceneNode(scene, {
            dispatchEvent: async payload => {
              const nextScene = await context.host.dispatchRustAppletEvent(rustAppletId, payload);
              renderScene(nextScene.scene);
            }
          })
        );
      };

      unsubscribeScene = context.host.onRustAppletScene(payload => {
        if (payload?.appletId !== rustAppletId) {
          return;
        }

        renderScene(payload.scene);
      });

      unsubscribeError = context.host.onRustAppletError(payload => {
        if (payload?.appletId !== rustAppletId) {
          return;
        }

        root.innerHTML = "";
        root.append(
          createStatusMessage({
            title: "Rust applet failed",
            body: payload?.message ?? "Unknown applet error",
            tone: "error"
          })
        );
      });

      const model = await context.host.loadRustApplet(rustAppletId, {
        role: context.windowRole
      });

      renderScene(model.scene);
    } catch (error) {
      if (unsubscribeScene) {
        unsubscribeScene();
      }
      if (unsubscribeError) {
        unsubscribeError();
      }
      root.innerHTML = "";
      root.append(
        createStatusMessage({
          title: "Rust applet failed",
          body: error?.message ?? String(error),
          tone: "error"
        })
      );
    }
  }
};
