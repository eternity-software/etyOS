#!/bin/sh

CURRENT_UID="$(id -u)"
if [ "$CURRENT_UID" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
  YAWC_USER_NAME="$SUDO_USER"
else
  YAWC_USER_NAME="$(id -un)"
fi

YAWC_USER_HOME=""
if command -v getent >/dev/null 2>&1; then
  YAWC_USER_HOME="$(getent passwd "$YAWC_USER_NAME" | awk -F: '{print $6}')"
fi
if [ -z "$YAWC_USER_HOME" ]; then
  YAWC_USER_HOME="${HOME:-}"
fi

if command -v cargo >/dev/null 2>&1; then
  YAWC_CARGO_BIN="$(command -v cargo)"
  export YAWC_USER_NAME YAWC_USER_HOME YAWC_CARGO_BIN
  return 0 2>/dev/null || exit 0
fi

if [ -n "$YAWC_USER_HOME" ] && [ -f "$YAWC_USER_HOME/.cargo/env" ]; then
  # shellcheck source=/dev/null
  . "$YAWC_USER_HOME/.cargo/env"
fi

if [ -n "$YAWC_USER_HOME" ] && [ -d "$YAWC_USER_HOME/.cargo/bin" ]; then
  export PATH="$YAWC_USER_HOME/.cargo/bin:$PATH"
fi

if command -v cargo >/dev/null 2>&1; then
  YAWC_CARGO_BIN="$(command -v cargo)"
  export YAWC_USER_NAME YAWC_USER_HOME YAWC_CARGO_BIN
  return 0 2>/dev/null || exit 0
fi

echo "error: cargo could not be found; checked PATH and $YAWC_USER_HOME/.cargo/bin" >&2
return 1 2>/dev/null || exit 1
