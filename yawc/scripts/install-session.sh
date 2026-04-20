#!/bin/sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
PREFIX="${PREFIX:-/usr/local}"
SESSION_DIR="${YAWC_SESSION_DIR:-/usr/share/wayland-sessions}"
BUILD_BINARY="$ROOT_DIR/target/debug/yawc"
INSTALLED_BINARY="$PREFIX/bin/yawc"
INSTALLED_LAUNCHER="$PREFIX/bin/yawc-session"
DESKTOP_TEMPLATE="$ROOT_DIR/packaging/yawc.desktop.in"
DESKTOP_FILE="$SESSION_DIR/yawc.desktop"

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
  "$ROOT_DIR/scripts/build-tty.sh"
fi

if [ ! -x "$BUILD_BINARY" ]; then
  echo "error: expected built tty binary at $BUILD_BINARY" >&2
  exit 1
fi

tmp_desktop="$(mktemp)"
tmp_launcher="$(mktemp)"
sed "s|@PREFIX@|$PREFIX|g" "$DESKTOP_TEMPLATE" > "$tmp_desktop"
awk -v repo_dir="$ROOT_DIR" -v yawc_binary="$INSTALLED_BINARY" '
  NR == 2 {
    print "YAWC_BINARY=\"${YAWC_BINARY:-" yawc_binary "}\""
    print "YAWC_REPO_DIR=\"${YAWC_REPO_DIR:-" repo_dir "}\""
  }
  { print }
' "$ROOT_DIR/scripts/yawc-session.sh" > "$tmp_launcher"

$SUDO install -Dm755 "$BUILD_BINARY" "$INSTALLED_BINARY"
$SUDO install -Dm755 "$tmp_launcher" "$INSTALLED_LAUNCHER"
$SUDO install -Dm644 "$tmp_desktop" "$DESKTOP_FILE"
rm -f "$tmp_desktop" "$tmp_launcher"

if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$DESKTOP_FILE" || true
fi

echo "Installed YAWC login session:"
echo "  binary:  $INSTALLED_BINARY"
echo "  launcher: $INSTALLED_LAUNCHER"
echo "  session: $DESKTOP_FILE"
echo ""
echo "Log out, open the session selector on the login screen, and choose YAWC."
