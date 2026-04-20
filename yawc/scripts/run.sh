#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
LOCAL_LIB_DIR="$ROOT_DIR/.local-lib"

# shellcheck source=/dev/null
. "$ROOT_DIR/scripts/rust-env.sh"

mkdir -p "$LOCAL_LIB_DIR"
ln -sf /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0 "$LOCAL_LIB_DIR/libxkbcommon.so"

cd "$ROOT_DIR"
exec env \
  HOME="$YAWC_USER_HOME" \
  USER="$YAWC_USER_NAME" \
  LOGNAME="$YAWC_USER_NAME" \
  RUSTFLAGS="-L native=$LOCAL_LIB_DIR" \
  "$YAWC_CARGO_BIN" run -- --winit "$@"
