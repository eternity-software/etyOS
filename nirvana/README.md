# Nirvana

Nirvana is the UI engine for `etyOS` system applets.

Current scope:

- Electron runtime for shell UI rendering
- internal `ui-kit` with reusable low-level primitives
- generic schema renderer for Rust-driven applets
- separate Rust-driven showcase applet that demonstrates the available components
- persistent Rust applet lifecycle with window open/close notifications
- typed protocol crate plus JS-side validation for scene payloads and callback events

This directory is intentionally independent from `yawc/`.

## Structure

- `src/main/`
  Electron entrypoint, preload bridge, protocol validation, and Rust applet runtime
- `src/renderer/runtime/`
  Nirvana applet host, registry, and schema renderer
- `src/renderer/ui-kit/`
  low-level UI primitives and theme tokens
- `src/renderer/applets/`
  renderer-side applet hosts rendered by Nirvana
- `applets/`
  Rust applet processes that provide scenes to Nirvana
- `applets/registry.json`
  allowlisted Rust applets that Nirvana is allowed to spawn
- `crates/nirvana-protocol/`
  shared Rust protocol types for requests and scene responses

## Run

```bash
cd /home/serio/etyOS/nirvana
./scripts/dev.sh
```

If you want direct package-manager access with the locally installed Node runtime:

```bash
cd /home/serio/etyOS/nirvana
./scripts/npm.sh install
./scripts/npm.sh run dev
```

Codex installed a local Node.js runtime into `/home/serio/etyOS/.tools/node/` because system-wide `apt` installation required a `sudo` password in this environment.

## Wayland

Nirvana is configured to run as a native Wayland app on Linux.

- the runtime appends `--ozone-platform=wayland`
- the launch script refuses to start if `WAYLAND_DISPLAY` is missing
- the launch script unsets `DISPLAY` to prevent accidental XWayland fallback

If Chromium's GPU process is unstable in your current session, you can force software rendering:

```bash
cd /home/serio/etyOS/nirvana
NIRVANA_DISABLE_GPU=1 ./scripts/dev.sh
```

This fallback is opt-in only. `./scripts/dev.sh` does not disable GPU acceleration by default.

## Security Model

Nirvana is set up so that the renderer stays a UI host, not a privileged shell backend.

- `contextIsolation: true`
- `sandbox: true`
- `nodeIntegration: false`
- denied `window.open`, navigation, and webview attachment
- denied runtime permission requests
- preload exposes a narrow frozen API instead of raw Electron primitives
- Rust applets are spawnable only from the allowlisted `applets/registry.json`
- scene payloads and callback payloads are validated before they cross process boundaries

## Rust Applets

Rust applets are persistent child processes.

- Nirvana notifies them when a window is opened or closed
- interactive components send callbacks back into the owning Rust applet
- when the last attached window closes, the applet is expected to shut down
- Nirvana enforces a graceful timeout and kills stuck applets if they do not exit

`NIRVANA_APPLET_MODE=auto` is the default:

- if `applets/<name>/target/release/<binary>` exists, Nirvana runs that binary directly
- otherwise it falls back to `cargo run --quiet --manifest-path ...`

If you want the runtime to use prebuilt binaries only:

```bash
cd /home/serio/etyOS/nirvana
cargo build --release --manifest-path applets/uikit-showcase/Cargo.toml
NIRVANA_APPLET_MODE=release ./scripts/dev.sh
```
