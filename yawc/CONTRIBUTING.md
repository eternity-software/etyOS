# Contributing to YAWC

Thanks for helping build YAWC as part of the `etyOS` / `etyDE` stack.

## Development Flow

1. Format and check the project:

   ```bash
   cargo fmt
   cargo check
   ```

2. Build the default nested compositor:

   ```bash
   ./scripts/build.sh
   ```

3. Run the compositor:

   ```bash
   ./scripts/run.sh
   ```

4. Run the smoke test launcher:

   ```bash
   ./scripts/run-test.sh
   ```

5. For standalone session work, rebuild the installed session binary:

   ```bash
   ./scripts/dev-update-session.sh
   ```

   Then relogin from the display manager and choose `YAWC`.

## Feature Testing Notes

- Window management changes should be tested with both server-side decorated apps, such as Konsole or OBS, and client-side decorated apps, such as VS Code.
- Portal and screen capture changes should be tested with OBS using `Screen Capture (PipeWire)`.
- Standalone backend changes should be tested through the SDDM/login-session path where possible, not only through nested `winit`.
- Config changes should be tested by editing `~/.config/yawc/config` while YAWC is running and confirming the next relevant input observes the new setting.

## Reporting Changes

When contributing, please include:

- what changed
- why it changed
- how it was tested
- any known limitations or follow-up work

## Scope

YAWC is a system compositor, not just a demo app.
Please prefer changes that keep the code modular, real, and maintainable.
Avoid hidden dependencies on KDE or GNOME portal backends unless the project intentionally decides to add one as a documented fallback.
