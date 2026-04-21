# <div align="center">YAWC</div>

<div align="center">
  <img src="./yawc_logo.png" alt="YAWC logo" width="140" />
  <p></p>
  <h3><strong>Yet Another Wayland Compositor</strong></h3>
  <p>Wayland compositor for <code>etyOS</code> and <code>etyDE</code>.</p>
</div>


## Overview

YAWC is the compositor foundation for the `etyOS` operating system and `etyDE` desktop environment.

YAWC provides a modular Wayland compositor core that can run nested for development or as a standalone Wayland session from a display manager.

YAWC is not based on KDE or GNOME compositor libraries. It uses Smithay and Wayland protocols directly, with `xdg-desktop-portal-wlr` used as the ScreenCast/Screenshot portal bridge for portal-aware apps such as OBS.

## Concept

YAWC is a compositor-first desktop foundation. It owns application windows, server-side decorations, input routing, frame rendering, session environment setup, and compositor-side desktop protocols.

The design favors direct Wayland/Smithay integration over borrowing behavior from another desktop environment. Runtime helpers such as `xdg-desktop-portal-wlr` are used as protocol bridges where appropriate, while YAWC remains the compositor and session owner.

## Capabilities

- Wayland socket setup and client lifecycle
- `xdg-shell` toplevel window support
- `xdg-shell` popup tracking and unconstraining
- `xdg-decoration` handling with server-side decorations
- YAWC-owned `wlr-screencopy` support for portal screen capture
- explicit window tracking with title and `app_id` metadata
- focus, raise-on-click, and active window state
- new windows raised above existing windows and centered by default
- move, resize, maximize, snap, restore, minimize, fullscreen, close, and force-kill handling
- close, minimize, and maximize SSD button handling in classic button mode
- buttonless gesture mode: right-click titlebar to close and double-click titlebar to maximize/restore
- live-reloaded configurable compositor hotkeys
- default window hotkeys: `Super+Up`, `Super+Left`, `Super+Right`, `Super+W`, `Super+Q`, `Ctrl+Alt+F`, `Ctrl+Alt+M`
- keyboard layout switching with `Alt+Shift`
- keyboard, pointer, and scroll input routing
- seat, clipboard/data-device, and drag-and-drop plumbing
- single-output nested compositor backend via `winit`
- standalone backend with `libseat`, `udev`, `libinput`, and single-output DRM/KMS scanout
- themed cursor loading for the standalone backend with fallback cursor rendering
- custom GPU-rendered window decorations
- rounded client corners and blurred titlebar styling
- popup, geometry, decoration, and close animations with live tuning
- desktop wallpaper support
- generated YAWC portal configuration for standalone sessions
- embedded host-window branding icon
- tracing-based logging

## Architecture

The repository is intentionally modular so the compositor can grow into a full system component instead of collapsing into one large `main.rs`.

### Core

- `src/main.rs`
  Entry point, tracing setup, startup command handling, session environment export, and event loop bootstrap.
- `src/state.rs`
  Global compositor state, Wayland socket setup, seat creation, popup manager, screencopy state, config reloads, and Smithay protocol state.
- `src/window.rs`
  Window tracking, fullscreen/maximize/snap/minimize geometry, restore geometry, animation state, frame geometry, hit testing, and decoration interaction regions.
- `src/cursor.rs`
  Cursor-shape mapping used by the nested and standalone backends.
- `src/screencopy.rs`
  `wlr-screencopy` protocol implementation used by `xdg-desktop-portal-wlr` for OBS/PipeWire screen capture.

### Protocol and Shell

- `src/shell/compositor.rs`
  Surface commit handling, SHM integration, and resize commit coordination.
- `src/shell/xdg.rs`
  Toplevels, popups, metadata updates, popup unconstraining, move/resize/maximize/minimize/fullscreen requests.
- `src/shell/decoration.rs`
  `xdg-decoration` policy.
- `src/shell/seat.rs`
  Seat, output, cursor-shape, data-device, and drag-and-drop plumbing.

### Interaction and Rendering

- `src/input.rs`
  Keyboard/pointer input dispatch, focus changes, pointer cursor updates, decoration clicks, gesture controls, and window hotkeys.
- `src/config.rs`
  Live-reloaded compositor config, hotkey parsing, keyboard config, animation tuning, and window-control mode.
- `src/grabs/`
  Dedicated move and resize grab implementations.
- `src/render.rs`
  Wallpaper rendering, custom frame composition, titlebar blur path, rounded content clipping, app icon lookup, title rendering, animation rendering, and screencopy readback.
- `src/backend/winit.rs`
  Nested host window setup and render loop integration.
- `src/backend/tty_udev.rs`
  Standalone backend bootstrap for `libseat`, `udev`, `libinput`, DRM/KMS scanout, themed cursor rendering, and screencopy capture.

## Quick Start

```bash
cd /home/serio/etyOS/yawc
./scripts/build.sh
./scripts/run-test.sh
```

`run-test.sh` launches YAWC and opens two Wayland clients inside it:

- `firefox`
- `konsole`

## Debian Dependencies

For Debian 13 (`trixie`) or newer:

```bash
sudo apt update
sudo apt install -y \
  cargo rustc pkg-config \
  libwayland-dev libxkbcommon-dev wayland-protocols \
  libegl1-mesa-dev libgles2-mesa-dev libudev-dev \
  fonts-noto-core firefox konsole
```

Notes:

- `fonts-noto-core` is used for titlebar text rendering.
- `firefox` and `konsole` are only needed for the default debug launcher.
- the helper scripts also create a local `libxkbcommon` linker workaround for this machine layout.

For the standalone backend, install the compositor device stack:

```bash
sudo apt install -y \
  libinput-dev libgbm-dev libdrm-dev libseat-dev seatd
```

And typically:

```bash
sudo systemctl enable --now seatd
```

If you do not want to install these packages system-wide, YAWC can bootstrap a local sysroot instead:

```bash
./scripts/bootstrap-standalone-deps.sh
```

This downloads the required Debian packages into `yawc/.sysroot/` and lets the standalone build scripts use them locally.

## Runtime Packages For etyOS Images

YAWC itself does not depend on KDE or GNOME portal backends. For a complete etyOS / etyDE image, include the runtime packages listed in:

```text
packaging/debian-runtime-packages.txt
```

Most importantly, OBS and other portal-aware screen capture clients need:

```bash
sudo apt install -y \
  xdg-desktop-portal xdg-desktop-portal-wlr \
  pipewire wireplumber
```

`xdg-desktop-portal-wlr` bridges the standard desktop portal ScreenCast API to YAWC's screencopy protocol support.
YAWC session scripts generate YAWC-specific portal preferences that route ScreenCast and Screenshot through that bridge while leaving unsupported portal interfaces disabled instead of silently depending on KDE or GNOME backends.

## Scripts

### Build

```bash
./scripts/build.sh
```

### Run

```bash
./scripts/run.sh
```

Runs YAWC as a nested compositor inside the current desktop session.
The nested host window uses the embedded `yawc_logo.png` as its app icon.

To start a client inside YAWC directly:

```bash
./scripts/run.sh --command foot
```

Any Wayland client command can be used in place of `foot`.

### Test Run

```bash
./scripts/run-test.sh
```

Recommended smoke-test path during `etyOS` / `etyDE` development.

### Standalone Build

```bash
./scripts/build-standalone.sh
```

Builds YAWC with the standalone backend feature set.
If `yawc/.sysroot/` exists, this script prefers the local sysroot automatically.

### Standalone Run

```bash
./scripts/run-standalone.sh
```

Compatibility wrapper around `start-standalone.sh`.
It performs the same active-VT standalone launch.

If you really want the unsafe direct backend path for development, use the internal script explicitly:

```bash
YAWC_ALLOW_DIRECT_STANDALONE_RUN=1 ./scripts/run-standalone-direct.sh
```

Important:

- this path targets a single output
- if you launch it from inside an already active desktop session on the same GPU, DRM ownership may be busy

### Login Screen Session

YAWC can be installed as a selectable Wayland session for display managers such as SDDM.
This is the preferred standalone path when you want to choose YAWC from the login screen instead of hand-launching it from a Linux VT.

```bash
./scripts/install-session.sh
```

The installer builds the standalone binary and installs:

- `/usr/local/bin/yawc`
- `/usr/local/bin/yawc-session`
- `/usr/share/wayland-sessions/yawc.desktop`

After installation, log out, open the session selector on the login screen, and choose `YAWC`.
The session launcher starts YAWC with the standalone backend and opens the first available terminal emulator.
Login-screen session output is saved to `~/.local/state/yawc/session.log`.

Fast development loop from inside a running YAWC session:

```bash
cd /home/serio/etyOS/yawc
./scripts/dev-update-session.sh
```

This rebuilds and replaces the installed session binary without stopping the currently running process.
Then press `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc` to return to the login screen and choose `YAWC` again.

## Configuration

YAWC creates a config file on startup if one does not already exist:

```text
~/.config/yawc/config
```

Hotkeys are reloaded automatically when keyboard input is handled, so after editing the file the next key press uses the new bindings without logging out or restarting YAWC.

Default config:

```text
maximize = Super+Up
snap_left = Super+Left
snap_right = Super+Right
fullscreen = Ctrl+Alt+F
minimize = Ctrl+Alt+M
close = Super+W
kill = Super+Q
layout_switch = Alt+Shift

keyboard_layouts = us
keyboard_model =
keyboard_variant =
keyboard_options =

animations = true
popup_animation_ms = 180
geometry_animation_ms = 220
decoration_animation_ms = 140
close_animation_ms = 220

window_controls = gestures
```

Pressing the same snap binding again restores the window to the position and size it had before snapping.
`Super+Up` toggles maximize/restore. `Super+W` asks the focused client to close. `Super+Q` force-kills the process associated with the focused window.

Supported modifiers are `Super`, `Ctrl`, `Alt`, and `Shift`. Supported keys are `Up`, `Down`, `Left`, `Right`, `F`, `M`, `Q`, and `W`. Set a binding to `none` to disable it.

`keyboard_layouts` is an XKB comma-separated layout list such as `us,ru`. `layout_switch = Alt+Shift` cycles through configured layouts.

`window_controls = gestures` hides titlebar buttons. In gesture mode, right-clicking the titlebar arms a close action and double-clicking the titlebar toggles maximize. Use `window_controls = buttons` or `window_controls = windows` for classic close/maximize/minimize buttons.

Animation values are in milliseconds and reload live. Set `animations = false` to disable popup, geometry, decoration, and close animations.

Useful session overrides:

```bash
YAWC_STARTUP_COMMAND=foot yawc-session
YAWC_DRM_LEGACY=1 yawc-session
YAWC_SKIP_PORTAL_CONFIG=1 yawc-session
YAWC_SESSION_LOG=/tmp/yawc-session.log yawc-session
RUST_LOG=yawc=trace,smithay=debug yawc-session
```

To remove the login session entry:

```bash
./scripts/uninstall-session.sh
```

### Separate VT Session

```bash
./scripts/start-standalone.sh
```

This is the manual standalone launch path for backend development.
On a single-GPU setup, stop the active desktop session first so YAWC can become DRM master cleanly:

```bash
# Save your work in Plasma first, then switch to a Linux VT with Ctrl+Alt+F3.
sudo systemctl stop sddm
cd /home/serio/etyOS/yawc
./scripts/start-standalone.sh
```

When you are done testing, stop YAWC with `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc`, then start the display manager again if needed:

```bash
sudo systemctl start sddm
```

Switch to a real Linux VT first, for example `Ctrl+Alt+F3`, log in as your normal user, then run this script without `sudo`.
It builds the standalone binary, runs YAWC on the current active VT through `dbus-run-session`, and autostarts a terminal inside YAWC.
It is safe to run from the repository root or from the `scripts/` directory.
All output is printed to the VT and saved automatically to `logs/standalone-session.log`.
The standalone launcher defaults to Smithay's legacy DRM path (`SMITHAY_USE_LEGACY=1`) unless `YAWC_DRM_LEGACY=0` is set.
The launcher refuses to start while an active desktop compositor such as KWin/Plasma is still running, because that leaves DRM ownership ambiguous and can show stale Plasma framebuffers instead of a clean YAWC frame.

Useful variations:

```bash
./scripts/start-standalone.sh --command foot
YAWC_STANDALONE_LOG=/tmp/yawc-standalone.log ./scripts/start-standalone.sh
YAWC_DRM_LEGACY=0 ./scripts/start-standalone.sh
YAWC_ALLOW_ACTIVE_DESKTOP=1 ./scripts/start-standalone.sh
```

`YAWC_ALLOW_ACTIVE_DESKTOP=1` is only for deliberate separate-GPU experiments.
Do not use it for the normal single-GPU Plasma-to-VT test path.

Emergency shortcuts in standalone mode:

- `Ctrl+Alt+F1` ... `Ctrl+Alt+F12` asks logind/libseat to switch VT.
- `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc` stops YAWC.

The `openvt` handoff path is available for experiments:

```bash
YAWC_USE_OPENVT=1 ./scripts/start-standalone.sh
YAWC_USE_OPENVT=1 YAWC_STANDALONE_VT=9 ./scripts/start-standalone.sh
```

The default path intentionally avoids `sudo/openvt`, because logind/libseat permissions are most reliable when the compositor starts directly from the active user VT.

The older `*-tty*.sh` helper names remain as compatibility wrappers around the `*-standalone*.sh` scripts.

## Logging

Default logging is `info`.

For verbose logs:

```bash
RUST_LOG=yawc=trace,smithay=debug ./scripts/run.sh --command foot
```

For installed login-screen sessions, logs are written to:

```text
~/.local/state/yawc/session.log
```

Portal debugging is usually done through the user journal:

```bash
journalctl --user -b -u xdg-desktop-portal -u xdg-desktop-portal-wlr --no-pager
```

## OBS And Screen Capture

YAWC implements the compositor side of `zwlr_screencopy_manager_v1`. In a complete session, OBS talks to `xdg-desktop-portal`, which talks to `xdg-desktop-portal-wlr`, which then captures frames from YAWC through screencopy and publishes them through PipeWire.

Required runtime packages are listed in `packaging/debian-runtime-packages.txt`.

If OBS does not show a PipeWire screen capture source, check that `xdg-desktop-portal-wlr` is installed and that the session was started through `yawc-session` so the generated YAWC portal files and environment are present.

If OBS shows a PipeWire source but capture fails, inspect:

```bash
tail -300 ~/.local/state/yawc/session.log
journalctl --user -b -u xdg-desktop-portal -u xdg-desktop-portal-wlr --no-pager
```
