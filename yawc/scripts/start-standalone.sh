#!/bin/sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
SCRIPT_PATH="$SCRIPT_DIR/start-standalone.sh"
LOG_DIR="$ROOT_DIR/logs"
LOG_FILE="${YAWC_STANDALONE_LOG:-${YAWC_TTY_LOG:-$LOG_DIR/standalone-session.log}}"
VT_NUMBER="${YAWC_STANDALONE_VT:-${YAWC_TTY_VT:-8}}"
LOCAL_LIB_DIR="$ROOT_DIR/.local-lib"
SYSROOT_ROOT="$ROOT_DIR/.sysroot/root"
BINARY_PATH="$ROOT_DIR/target/debug/yawc"
LAUNCHER_PATH="$ROOT_DIR/scripts/launch-standalone-binary.sh"

active_desktop_processes() {
  found=""
  for process in \
    kwin_wayland \
    kwin_x11 \
    plasmashell \
    gnome-shell \
    mutter \
    weston \
    sway \
    Hyprland \
    gamescope
  do
    if pgrep -x "$process" >/dev/null 2>&1; then
      if [ -n "$found" ]; then
        found="$found, $process"
      else
        found="$process"
      fi
    fi
  done

  printf '%s\n' "$found"
}

refuse_with_active_desktop_if_needed() {
  if [ "${YAWC_ALLOW_ACTIVE_DESKTOP:-0}" = "1" ]; then
    echo "warning: YAWC_ALLOW_ACTIVE_DESKTOP=1 set; skipping active desktop safety check" >&2
    return
  fi

  desktop_processes="$(active_desktop_processes)"
  if [ -z "$desktop_processes" ]; then
    return
  fi

  echo "error: active desktop compositor/session detected: $desktop_processes" >&2
  echo "YAWC standalone needs DRM master on the target GPU." >&2
  echo "Running it while Plasma/KWin is still active can show stale Plasma framebuffers or freeze input/output." >&2
  echo "" >&2
  echo "Safe test path:" >&2
  echo "  1. Save your work in Plasma." >&2
  echo "  2. Switch to a Linux VT: Ctrl+Alt+F3." >&2
  echo "  3. Stop the display manager: sudo systemctl stop sddm" >&2
  echo "  4. Log in on the VT and run: cd $ROOT_DIR && ./scripts/start-standalone.sh" >&2
  echo "" >&2
  echo "Only override with YAWC_ALLOW_ACTIVE_DESKTOP=1 if you are intentionally testing on a separate GPU." >&2
  exit 1
}

if [ "${YAWC_STANDALONE_LOG_ACTIVE:-${YAWC_TTY_LOG_ACTIVE:-0}}" != "1" ]; then
  mkdir -p "$LOG_DIR"
  : > "$LOG_FILE"
  export YAWC_STANDALONE_LOG_ACTIVE=1
  export YAWC_STANDALONE_LOG="$LOG_FILE"
  export YAWC_TTY_LOG_ACTIVE=1
  export YAWC_TTY_LOG="$LOG_FILE"
  status_file="$(mktemp)"
  (
    set +e
    "$SCRIPT_PATH" "$@"
    status="$?"
    printf '%s\n' "$status" > "$status_file"
    exit "$status"
  ) 2>&1 | tee "$LOG_FILE"
  status="$(cat "$status_file" 2>/dev/null || printf '1')"
  rm -f "$status_file"
  exit "$status"
fi

echo "YAWC standalone launch log: $LOG_FILE"

if [ "$(id -u)" -eq 0 ]; then
  RUN_AS_USER="${YAWC_RUN_AS_USER:-${SUDO_USER:-serio}}"
else
  RUN_AS_USER="$(id -un)"
fi

USER_HOME=""
if command -v getent >/dev/null 2>&1; then
  USER_HOME="$(getent passwd "$RUN_AS_USER" | awk -F: '{print $6}')"
fi
if [ -z "$USER_HOME" ]; then
  USER_HOME="/home/$RUN_AS_USER"
fi

refuse_with_active_desktop_if_needed

has_command_arg=0
for arg in "$@"; do
  case "$arg" in
    -c|--command)
      has_command_arg=1
      break
      ;;
  esac
done

if [ "$has_command_arg" -eq 0 ]; then
  terminal="$("$ROOT_DIR/scripts/default-terminal.sh")"
  set -- --command "$terminal" "$@"
fi

mkdir -p "$LOCAL_LIB_DIR"
ln -sf /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0 "$LOCAL_LIB_DIR/libxkbcommon.so"

if [ "${YAWC_SKIP_STANDALONE_BUILD:-${YAWC_SKIP_TTY_BUILD:-0}}" != "1" ]; then
  echo "Building YAWC standalone binary before VT handoff..." >&2
  if [ "$(id -u)" -eq 0 ] && [ "$RUN_AS_USER" != "root" ]; then
    sudo -u "$RUN_AS_USER" env -u SUDO_USER \
      HOME="$USER_HOME" \
      USER="$RUN_AS_USER" \
      LOGNAME="$RUN_AS_USER" \
      "$ROOT_DIR/scripts/build-standalone.sh"
  else
    "$ROOT_DIR/scripts/build-standalone.sh"
  fi
fi

if [ ! -x "$BINARY_PATH" ]; then
  echo "error: expected built binary at $BINARY_PATH" >&2
  exit 1
fi

EXTRA_LD_LIBRARY_PATH="$LOCAL_LIB_DIR"
if [ -d "$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu" ]; then
  EXTRA_LD_LIBRARY_PATH="$EXTRA_LD_LIBRARY_PATH:$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu"
fi

if ! command -v dbus-run-session >/dev/null 2>&1; then
  echo "error: dbus-run-session is required for a standalone YAWC session" >&2
  exit 1
fi

if [ "${YAWC_USE_OPENVT:-0}" != "1" ]; then
  if [ "$(id -u)" -eq 0 ]; then
    echo "error: do not run the default standalone launcher with sudo/root" >&2
    echo "hint: switch to a real Linux VT, log in as $RUN_AS_USER, then run ./scripts/start-standalone.sh" >&2
    echo "hint: set YAWC_USE_OPENVT=1 only if you intentionally want the experimental openvt path" >&2
    exit 1
  fi

  current_tty="$(tty 2>/dev/null || true)"
  case "$current_tty" in
    /dev/tty[0-9]*)
      ;;
    *)
      echo "error: start-standalone.sh must be run from a real Linux VT, e.g. Ctrl+Alt+F3" >&2
      echo "hint: current terminal is '$current_tty', not /dev/ttyN" >&2
      exit 1
      ;;
  esac

  echo "Launching YAWC on current VT as ${RUN_AS_USER}..." >&2
  exec "$LAUNCHER_PATH" "$BINARY_PATH" "$USER_HOME" "$RUN_AS_USER" "$EXTRA_LD_LIBRARY_PATH" "$@"
fi

if [ "$(id -u)" -ne 0 ]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: sudo is required for YAWC_USE_OPENVT=1" >&2
    exit 1
  fi

  exec sudo env \
    YAWC_RUN_AS_USER="$RUN_AS_USER" \
    YAWC_STANDALONE_VT="$VT_NUMBER" \
    YAWC_STANDALONE_LOG="$LOG_FILE" \
    YAWC_STANDALONE_LOG_ACTIVE=1 \
    YAWC_SKIP_STANDALONE_BUILD=1 \
    YAWC_TTY_VT="$VT_NUMBER" \
    YAWC_TTY_LOG="$LOG_FILE" \
    YAWC_TTY_LOG_ACTIVE=1 \
    YAWC_SKIP_TTY_BUILD=1 \
    YAWC_USE_OPENVT=1 \
    sh "$SCRIPT_PATH" "$@"
fi

if ! command -v openvt >/dev/null 2>&1; then
  echo "error: openvt is required for YAWC_USE_OPENVT=1" >&2
  exit 1
fi

echo "Launching YAWC on VT${VT_NUMBER} as ${RUN_AS_USER} via experimental openvt path..." >&2
echo "Use Ctrl+Alt+F${VT_NUMBER} to switch if your environment does not auto-focus the VT." >&2

if [ "$RUN_AS_USER" != "root" ]; then
  exec openvt -c "$VT_NUMBER" -s -f -- \
    sudo -u "$RUN_AS_USER" env -u SUDO_USER \
      HOME="$USER_HOME" \
      USER="$RUN_AS_USER" \
      LOGNAME="$RUN_AS_USER" \
      "$LAUNCHER_PATH" "$BINARY_PATH" "$USER_HOME" "$RUN_AS_USER" "$EXTRA_LD_LIBRARY_PATH" "$@"
fi

exec openvt -c "$VT_NUMBER" -s -f -- \
  "$LAUNCHER_PATH" "$BINARY_PATH" "$USER_HOME" "$RUN_AS_USER" "$EXTRA_LD_LIBRARY_PATH" "$@"
