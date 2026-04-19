# <div align="center">YAWC</div>

<div align="center">
  <img src="./yawc_logo.png" alt="YAWC logo" width="140" />
  <p></p>
  <h3><strong>Yet Another Wayland Compositor</strong></h3>
  <p>Wayland compositor for <code>etyOS</code> and <code>etyDE</code>.</p>
</div>


## Overview

YAWC is the compositor foundation for the `etyOS` operating system and `etyDE` desktop environment.

The current codebase focuses on building a real, clean, extensible compositor core that can be iterated quickly in a nested development setup.

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
- performance and render-path hardening
