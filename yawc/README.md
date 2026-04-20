# <div align="center">YAWC</div>

<div align="center">
  <img src="./yawc_logo.png" alt="YAWC logo" width="140" />
  <p></p>
  <h3><strong>Yet Another Wayland Compositor</strong></h3>
  <p>Wayland compositor for <code>etyOS</code> and <code>etyDE</code>.</p>
</div>


## Overview

YAWC is the compositor foundation for the `etyOS` operating system and `etyDE` desktop environment.

The current codebase focuses on building a real, clean, extensible compositor core that can be iterated quickly in a nested development setup and brought up on a real VT when needed.

## What Works Right Now

- Wayland socket setup and client lifecycle
- `xdg-shell` toplevel window support
- `xdg-shell` popup tracking and unconstraining
- `xdg-decoration` handling with server-side decorations
- explicit window tracking with title and `app_id` metadata
- focus, raise-on-click, and active window state
- move and resize grabs
- close button handling
- keyboard, pointer, and scroll input routing
- seat and data-device skeleton
- single-output nested compositor backend via `winit`
- experimental `tty-udev` backend with `libseat`, `udev`, `libinput`, and single-output DRM/KMS scanout
- custom GPU-rendered window decorations
- rounded client corners and blurred titlebar styling
- desktop wallpaper support
- embedded host-window branding icon
- tracing-based logging

## Current Architecture

The repository is intentionally modular so the compositor can grow into a full system component instead of collapsing into one large `main.rs`.

### Core

- `src/main.rs`
  Entry point, tracing setup, startup command handling, and event loop bootstrap.
- `src/state.rs`
  Global compositor state, Wayland socket setup, seat creation, popup manager, and Smithay protocol state.
- `src/window.rs`
  Window tracking, frame geometry, hit testing, and decoration interaction regions.

### Protocol and Shell

- `src/shell/compositor.rs`
  Surface commit handling, SHM integration, and resize commit coordination.
- `src/shell/xdg.rs`
  Toplevels, popups, metadata updates, popup unconstraining, move/resize requests.
- `src/shell/decoration.rs`
  `xdg-decoration` policy.
- `src/shell/seat.rs`
  Seat, output, and data-device plumbing.

### Interaction and Rendering

- `src/input.rs`
  Keyboard/pointer input dispatch, focus changes, pointer cursor updates, decoration clicks.
- `src/grabs/`
  Dedicated move and resize grab implementations.
- `src/render.rs`
  Wallpaper rendering, custom frame composition, titlebar blur path, rounded content clipping, app icon lookup, title rendering.
- `src/backend/winit.rs`
  Nested host window setup and render loop integration.
- `src/backend/tty_udev.rs`
  Experimental standalone backend bootstrap for `libseat`, `udev`, and `libinput`.

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

For the experimental `tty-udev` backend, you will additionally need the standalone compositor stack:

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
./scripts/bootstrap-tty-deps.sh
```

This downloads the required Debian packages into `yawc/.sysroot/` and lets the tty build scripts use them locally.

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

### Experimental TTY Build

```bash
./scripts/build-tty.sh
```

Builds YAWC with the experimental `tty-udev` backend feature set.
If `yawc/.sysroot/` exists, this script prefers the local sysroot automatically.

### Experimental TTY Run

```bash
./scripts/run-tty.sh
```

Compatibility wrapper around `start-tty.sh`.
It performs the same active-TTY standalone launch.

If you really want the unsafe direct backend path for development, use the internal script explicitly:

```bash
YAWC_ALLOW_DIRECT_TTY_RUN=1 ./scripts/run-tty-direct.sh
```

Important:

- this does not replace the current nested backend
- this is still an experimental standalone path
- the current implementation targets a single output
- if you launch it from inside an already active desktop session on the same GPU, DRM ownership may be busy

### Login Screen Session

YAWC can be installed as a selectable Wayland session for display managers such as SDDM.
This is the preferred standalone path when you want to choose YAWC from the login screen instead of hand-launching it from a TTY.

```bash
./scripts/install-session.sh
```

The installer builds the `tty-udev` binary and installs:

- `/usr/local/bin/yawc`
- `/usr/local/bin/yawc-session`
- `/usr/share/wayland-sessions/yawc.desktop`

After installation, log out, open the session selector on the login screen, and choose `YAWC`.
The session launcher starts YAWC with the standalone `tty-udev` backend and opens the first available terminal emulator.
Login-screen session output is saved to `~/.local/state/yawc/session.log`.

Fast development loop from inside a running YAWC session:

```bash
cd /home/serio/etyOS/yawc
./scripts/dev-update-session.sh
```

This rebuilds and replaces the installed session binary without stopping the currently running process.
Then press `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc` to return to the login screen and choose `YAWC` again.

Useful session overrides:

```bash
YAWC_STARTUP_COMMAND=foot yawc-session
YAWC_DRM_LEGACY=1 yawc-session
YAWC_SESSION_LOG=/tmp/yawc-session.log yawc-session
RUST_LOG=yawc=trace,smithay=debug yawc-session
```

To remove the login session entry:

```bash
./scripts/uninstall-session.sh
```

### Separate VT Session

```bash
./scripts/start-tty.sh
```

This is the manual standalone launch path for backend development.
For the current single-GPU development setup, stop the active desktop session first so YAWC can become DRM master cleanly:

```bash
# Save your work in Plasma first, then switch to a TTY with Ctrl+Alt+F3.
sudo systemctl stop sddm
cd /home/serio/etyOS/yawc
./scripts/start-tty.sh
```

When you are done testing, stop YAWC with `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc`, then start the display manager again if needed:

```bash
sudo systemctl start sddm
```

Switch to a real Linux TTY first, for example `Ctrl+Alt+F3`, log in as your normal user, then run this script without `sudo`.
It builds the tty binary, runs YAWC on the current active TTY through `dbus-run-session`, and autostarts a terminal inside YAWC.
It is safe to run from the repository root or from the `scripts/` directory.
All output is printed to the TTY and saved automatically to `logs/tty-session.log`.
The standalone launcher currently defaults to Smithay's legacy DRM path (`SMITHAY_USE_LEGACY=1`) because this is safer on the current NVIDIA setup than the atomic path.
The launcher refuses to start while an active desktop compositor such as KWin/Plasma is still running, because that leaves DRM ownership ambiguous and can show stale Plasma framebuffers instead of a clean YAWC frame.

Useful variations:

```bash
./scripts/start-tty.sh --command foot
YAWC_TTY_LOG=/tmp/yawc-tty.log ./scripts/start-tty.sh
YAWC_DRM_LEGACY=0 ./scripts/start-tty.sh
YAWC_ALLOW_ACTIVE_DESKTOP=1 ./scripts/start-tty.sh
```

`YAWC_ALLOW_ACTIVE_DESKTOP=1` is only for deliberate separate-GPU experiments.
Do not use it for the normal single-GPU Plasma-to-TTY test path.

Emergency shortcuts in standalone mode:

- `Ctrl+Alt+F1` ... `Ctrl+Alt+F12` asks logind/libseat to switch VT.
- `Ctrl+Alt+Backspace` or `Ctrl+Alt+Esc` stops YAWC.

The old `openvt` handoff path is still available for experiments:

```bash
YAWC_USE_OPENVT=1 ./scripts/start-tty.sh
YAWC_USE_OPENVT=1 YAWC_TTY_VT=9 ./scripts/start-tty.sh
```

The default path intentionally avoids `sudo/openvt`, because logind/libseat permissions are most reliable when the compositor starts directly from the active user TTY.

`run-tty-session.sh` remains as a compatibility wrapper around `start-tty.sh`.

## Logging

Default logging is `info`.

For verbose logs:

```bash
RUST_LOG=yawc=trace,smithay=debug ./scripts/run.sh --command foot
```

## GitHub Publishing Notes

This repository is prepared for GitHub publication with:

- a project-focused README
- helper scripts for build and smoke-test runs
- GitHub issue templates
- a pull request template
- a basic Rust CI workflow

One important publication choice is still intentionally left to the repository owner:

- license selection

No license file has been added automatically because that is a real legal/project decision.

## Roadmap Direction

Planned next layers for YAWC as part of `etyOS` / `etyDE` include:

- multi-window desktop policy
- workspaces and window management rules
- shell surfaces beyond application windows
- compositor-owned system UI
- stronger session integration
- broader backend support
- full tty/udev scanout path
- performance and render-path hardening
