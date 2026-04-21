#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
LOCAL_LIB_DIR="$ROOT_DIR/.local-lib"
SYSROOT_ROOT="$ROOT_DIR/.sysroot/root"
LOCAL_PKG_CONFIG="$ROOT_DIR/scripts/pkg-config-local.sh"

# shellcheck source=/dev/null
. "$ROOT_DIR/scripts/rust-env.sh"

if [ -x "$SYSROOT_ROOT/usr/bin/pkg-config" ]; then
  PKG_CONFIG_CMD="$LOCAL_PKG_CONFIG"
  PKG_CONFIG_LIBDIR="$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu/pkgconfig"
  EXTRA_RUSTFLAGS="-L native=$LOCAL_LIB_DIR -L native=$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu -C link-arg=-fuse-ld=bfd"
  EXTRA_LD_LIBRARY_PATH="$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu"
elif command -v pkg-config >/dev/null 2>&1; then
  PKG_CONFIG_CMD="pkg-config"
  PKG_CONFIG_LIBDIR=""
  EXTRA_RUSTFLAGS="-L native=$LOCAL_LIB_DIR"
  EXTRA_LD_LIBRARY_PATH=""
else
  echo "error: pkg-config is required for the standalone backend" >&2
  echo "hint: either install system packages or run ./scripts/bootstrap-standalone-deps.sh" >&2
  exit 1
fi

mkdir -p "$LOCAL_LIB_DIR"
ln -sf /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0 "$LOCAL_LIB_DIR/libxkbcommon.so"

cd "$ROOT_DIR"
if [ -n "$PKG_CONFIG_LIBDIR" ]; then
  exec env \
    HOME="$YAWC_USER_HOME" \
    USER="$YAWC_USER_NAME" \
    LOGNAME="$YAWC_USER_NAME" \
    RUSTFLAGS="$EXTRA_RUSTFLAGS" \
    LD_LIBRARY_PATH="$EXTRA_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    PKG_CONFIG="$PKG_CONFIG_CMD" \
    PKG_CONFIG_LIBDIR="$PKG_CONFIG_LIBDIR" \
    C_INCLUDE_PATH="$SYSROOT_ROOT/usr/include:$SYSROOT_ROOT/usr/include/libdrm" \
    "$YAWC_CARGO_BIN" build --no-default-features --features standalone "$@"
fi

exec env \
  HOME="$YAWC_USER_HOME" \
  USER="$YAWC_USER_NAME" \
  LOGNAME="$YAWC_USER_NAME" \
  RUSTFLAGS="$EXTRA_RUSTFLAGS" \
  PKG_CONFIG="$PKG_CONFIG_CMD" \
  "$YAWC_CARGO_BIN" build --no-default-features --features standalone "$@"
