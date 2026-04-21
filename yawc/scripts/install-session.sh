#!/bin/sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
PREFIX="${PREFIX:-/usr/local}"
SESSION_DIR="${YAWC_SESSION_DIR:-/usr/share/wayland-sessions}"
BUILD_BINARY="$ROOT_DIR/target/debug/yawc"
INSTALLED_BINARY="$PREFIX/bin/yawc"
INSTALLED_LAUNCHER="$PREFIX/bin/yawc-session"
INSTALLED_PROBE_LAUNCHER="$PREFIX/bin/yawc-dmabuf-probe-session"
INSTALLED_GPU_DEBUG="$PREFIX/bin/yawc-debug-gpu-env"
DESKTOP_TEMPLATE="$ROOT_DIR/packaging/yawc.desktop.in"
PROBE_DESKTOP_TEMPLATE="$ROOT_DIR/packaging/yawc-dmabuf-probe.desktop.in"
DESKTOP_FILE="$SESSION_DIR/yawc.desktop"
PROBE_DESKTOP_FILE="$SESSION_DIR/yawc-dmabuf-probe.desktop"
INSTALL_DMABUF_PROBE="${YAWC_INSTALL_DMABUF_PROBE:-0}"

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif [ "${YAWC_INSTALL_WITHOUT_SUDO:-0}" = "1" ]; then
  SUDO=""
else
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: sudo is required to install the login session files" >&2
    exit 1
  fi
  SUDO="sudo"
fi

if [ "${YAWC_SKIP_BUILD:-0}" != "1" ]; then
  "$ROOT_DIR/scripts/build-standalone.sh"
fi

if [ ! -x "$BUILD_BINARY" ]; then
  echo "error: expected built standalone binary at $BUILD_BINARY" >&2
  exit 1
fi

tmp_desktop="$(mktemp)"
tmp_launcher="$(mktemp)"
sed "s|@PREFIX@|$PREFIX|g" "$DESKTOP_TEMPLATE" > "$tmp_desktop"
awk \
  -v repo_dir="$ROOT_DIR" \
  -v yawc_binary="$INSTALLED_BINARY" \
  -v gpu_vendor="${YAWC_GPU_VENDOR:-auto}" '
  NR == 2 {
    print "YAWC_BINARY=\"${YAWC_BINARY:-" yawc_binary "}\""
    print "YAWC_REPO_DIR=\"${YAWC_REPO_DIR:-" repo_dir "}\""
    print "YAWC_DEFAULT_GPU_VENDOR=\"${YAWC_DEFAULT_GPU_VENDOR:-" gpu_vendor "}\""
  }
  { print }
' "$ROOT_DIR/scripts/yawc-session.sh" > "$tmp_launcher"

$SUDO install -Dm755 "$BUILD_BINARY" "$INSTALLED_BINARY"
$SUDO install -Dm755 "$tmp_launcher" "$INSTALLED_LAUNCHER"
$SUDO install -Dm755 "$ROOT_DIR/scripts/debug-gpu-env.sh" "$INSTALLED_GPU_DEBUG"
$SUDO install -Dm644 "$tmp_desktop" "$DESKTOP_FILE"

if [ "$INSTALL_DMABUF_PROBE" = "1" ]; then
  tmp_probe_desktop="$(mktemp)"
  sed "s|@PREFIX@|$PREFIX|g" "$PROBE_DESKTOP_TEMPLATE" > "$tmp_probe_desktop"
  $SUDO install -Dm755 "$ROOT_DIR/scripts/yawc-dmabuf-probe-session.sh" "$INSTALLED_PROBE_LAUNCHER"
  $SUDO install -Dm644 "$tmp_probe_desktop" "$PROBE_DESKTOP_FILE"
  rm -f "$tmp_probe_desktop"
else
  $SUDO rm -f "$INSTALLED_PROBE_LAUNCHER" "$PROBE_DESKTOP_FILE"
fi

rm -f "$tmp_desktop" "$tmp_launcher"

if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$DESKTOP_FILE" || true
  if [ "$INSTALL_DMABUF_PROBE" = "1" ]; then
    desktop-file-validate "$PROBE_DESKTOP_FILE" || true
  fi
fi

echo "Installed YAWC login session:"
echo "  binary:  $INSTALLED_BINARY"
echo "  launcher: $INSTALLED_LAUNCHER"
echo "  gpu debug: $INSTALLED_GPU_DEBUG"
echo "  session: $DESKTOP_FILE"
if [ "$INSTALL_DMABUF_PROBE" = "1" ]; then
  echo "  dmabuf probe launcher: $INSTALLED_PROBE_LAUNCHER"
  echo "  dmabuf probe session: $PROBE_DESKTOP_FILE"
else
  echo "  dmabuf probe session: not installed (set YAWC_INSTALL_DMABUF_PROBE=1 to add it)"
fi
echo ""
echo "Log out, open the session selector on the login screen, and choose YAWC."
