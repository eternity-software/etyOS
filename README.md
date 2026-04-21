# etyOS

`etyOS` is the workspace for the etyOS operating system and etyDE desktop environment work.
`yawc` is the Wayland compositor that forms the desktop/session foundation.

## Concept

etyOS is built around etyDE, a Wayland-first desktop environment. YAWC provides the compositor layer: it owns windows, input routing, server-side decorations, animations, session startup, and the compositor-side protocols that desktop applications expect.

The repository contains the Rust/Smithay compositor in `yawc/`. YAWC can run nested for development or as a login-screen Wayland session through SDDM.

## Present Capabilities

- Server-side decorations with blur, rounded client clipping, and gesture or button controls.
- Custom cursors, animation tuning, live config reloads, and keyboard layout switching.
- Drag-and-drop/data-device plumbing.
- OBS/PipeWire screen capture through `xdg-desktop-portal-wlr` and YAWC screencopy support.
- Standalone hardware session startup through the YAWC login session.

## Quick Start

```bash
cd yawc
./scripts/build.sh
./scripts/run-test.sh
```

For the standalone login session:

```bash
cd yawc
./scripts/install-session.sh
```

Then log out and choose `YAWC` in the display manager session selector.

For the fast development loop from an installed YAWC session:

```bash
cd yawc
./scripts/dev-update-session.sh
```

Relogin from the display manager after rebuilding the installed compositor binary.

## Runtime Image Notes

YAWC is not based on KDE or GNOME libraries. A complete etyOS image should include the runtime packages listed in:

```text
yawc/packaging/debian-runtime-packages.txt
```

Important runtime pieces include `xdg-desktop-portal`, `xdg-desktop-portal-wlr`, `pipewire`, and `wireplumber` so portal-aware apps such as OBS can capture the screen out of the box.

## Documentation

See [yawc/README.md](yawc/README.md) for compositor architecture, dependency, configuration, and development details.
