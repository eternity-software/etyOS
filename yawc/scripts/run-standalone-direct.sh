#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
LOCAL_LIB_DIR="$ROOT_DIR/.local-lib"
SYSROOT_ROOT="$ROOT_DIR/.sysroot/root"
LOCAL_PKG_CONFIG="$ROOT_DIR/scripts/pkg-config-local.sh"

# shellcheck source=/dev/null
. "$ROOT_DIR/scripts/rust-env.sh"

if [ "${YAWC_ALLOW_DIRECT_STANDALONE_RUN:-${YAWC_ALLOW_DIRECT_TTY_RUN:-0}}" != "1" ]; then
  echo "error: direct standalone launch is disabled by default" >&2
  echo "hint: use ./scripts/start-standalone.sh so YAWC starts on a dedicated VT" >&2
  echo "hint: set YAWC_ALLOW_DIRECT_STANDALONE_RUN=1 only if you intentionally want the unsafe direct path" >&2
  exit 1
fi

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

HAS_COMMAND_ARG=0
for arg in "$@"; do
  case "$arg" in
    -c|--command)
      HAS_COMMAND_ARG=1
      break
      ;;
  esac
done

if [ "$HAS_COMMAND_ARG" -eq 0 ] && TERMINAL="$("$ROOT_DIR/scripts/default-terminal.sh" 2>/dev/null)"; then
  set -- --command "$TERMINAL" "$@"
fi

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
    "$YAWC_CARGO_BIN" run --no-default-features --features standalone -- --standalone "$@"
fi

exec env \
  HOME="$YAWC_USER_HOME" \
  USER="$YAWC_USER_NAME" \
  LOGNAME="$YAWC_USER_NAME" \
  RUSTFLAGS="$EXTRA_RUSTFLAGS" \
  PKG_CONFIG="$PKG_CONFIG_CMD" \
  "$YAWC_CARGO_BIN" run --no-default-features --features standalone -- --standalone "$@"
