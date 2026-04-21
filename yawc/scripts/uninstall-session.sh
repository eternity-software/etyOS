#!/bin/sh
set -eu

PREFIX="${PREFIX:-/usr/local}"
SESSION_DIR="${YAWC_SESSION_DIR:-/usr/share/wayland-sessions}"

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif [ "${YAWC_INSTALL_WITHOUT_SUDO:-0}" = "1" ]; then
  SUDO=""
else
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: sudo is required to uninstall the login session files" >&2
    exit 1
  fi
  SUDO="sudo"
fi

$SUDO rm -f "$SESSION_DIR/yawc.desktop"
$SUDO rm -f "$SESSION_DIR/yawc-dmabuf-probe.desktop"
$SUDO rm -f "$PREFIX/bin/yawc-session"
$SUDO rm -f "$PREFIX/bin/yawc-dmabuf-probe-session"
$SUDO rm -f "$PREFIX/bin/yawc-debug-gpu-env"
$SUDO rm -f "$PREFIX/bin/yawc"

echo "Removed YAWC login session files."
