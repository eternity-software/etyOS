#!/bin/sh
set -eu

state_home="${XDG_STATE_HOME:-${HOME:-/tmp}/.local/state}"
log_dir="${YAWC_LOG_DIR:-$state_home/yawc}"
mkdir -p "$log_dir"

probe_env="${YAWC_DMABUF_PROBE_ENV:-${HOME:-/tmp}/.config/yawc/dmabuf-probe-env}"
if [ -f "$probe_env" ]; then
  # shellcheck disable=SC1090
  . "$probe_env"
fi

probe_log="${YAWC_DMABUF_PROBE_LOG:-$log_dir/dmabuf-probe.log}"
eglinfo_log="${YAWC_DMABUF_EGLINFO_LOG:-$log_dir/dmabuf-probe-eglinfo.log}"
obs_log="${YAWC_DMABUF_OBS_LOG:-$log_dir/dmabuf-probe-obs.log}"
watchdog_timeout="${YAWC_DMABUF_PROBE_TIMEOUT:-20}"

: > "$probe_log"
: > "$eglinfo_log"
: > "$obs_log"

export YAWC_SESSION_LOG="${YAWC_SESSION_LOG:-$log_dir/dmabuf-probe-session.log}"
export YAWC_ENABLE_DMABUF_PROBE=1
export RUST_LOG="${RUST_LOG:-info,yawc=debug}"

case "${YAWC_DMABUF_PROBE_EGL_VENDOR:-default}" in
  mesa)
    egl_vendor_env="__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json"
    ;;
  nvidia)
    egl_vendor_env="__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json"
    ;;
  default | "")
    egl_vendor_env=""
    ;;
  *)
    echo "Unknown YAWC_DMABUF_PROBE_EGL_VENDOR=${YAWC_DMABUF_PROBE_EGL_VENDOR}" >> "$probe_log"
    egl_vendor_env=""
    ;;
esac

(
  sleep "$watchdog_timeout"
  {
    echo "YAWC dmabuf probe watchdog fired at $(date -Is 2>/dev/null || date)"
    echo "Stopping probe compositor so the display manager can recover"
  } >> "$probe_log"
  pkill -TERM -x yawc 2>/dev/null || true
  sleep 2
  pkill -KILL -x yawc 2>/dev/null || true
) &

probe_command="
  set -eu
  echo \"YAWC dmabuf probe started at \$(date -Is 2>/dev/null || date)\" >> '$probe_log'
  echo \"WAYLAND_DISPLAY=\${WAYLAND_DISPLAY:-}\" >> '$probe_log'
  echo \"EGL vendor override: ${YAWC_DMABUF_PROBE_EGL_VENDOR:-default}\" >> '$probe_log'
  echo \"Running eglinfo probe\" >> '$probe_log'
  if command -v eglinfo >/dev/null 2>&1; then
    env $egl_vendor_env EGL_PLATFORM=wayland eglinfo -p wayland > '$eglinfo_log' 2>&1 || true
  else
    echo 'eglinfo is not installed' > '$eglinfo_log'
  fi
  echo \"Running OBS renderer probe\" >> '$probe_log'
  if command -v obs >/dev/null 2>&1; then
    env $egl_vendor_env timeout 20s obs --verbose > '$obs_log' 2>&1 || true
  else
    echo 'obs is not installed' > '$obs_log'
  fi
  echo \"Probe complete at \$(date -Is 2>/dev/null || date)\" >> '$probe_log'
  sleep 5
"

export YAWC_STARTUP_COMMAND="$probe_command"

exec "${YAWC_SESSION_LAUNCHER:-/usr/local/bin/yawc-session}"
