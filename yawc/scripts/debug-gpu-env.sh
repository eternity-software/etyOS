#!/bin/sh
set -eu

section() {
  printf '\n== %s ==\n' "$1"
}

section "Session"
printf 'XDG_SESSION_TYPE=%s\n' "${XDG_SESSION_TYPE:-}"
printf 'WAYLAND_DISPLAY=%s\n' "${WAYLAND_DISPLAY:-}"
printf 'DISPLAY=%s\n' "${DISPLAY:-}"
printf 'XDG_CURRENT_DESKTOP=%s\n' "${XDG_CURRENT_DESKTOP:-}"
printf 'EGL_PLATFORM=%s\n' "${EGL_PLATFORM:-}"
printf 'GBM_BACKEND=%s\n' "${GBM_BACKEND:-}"
printf '__EGL_VENDOR_LIBRARY_FILENAMES=%s\n' "${__EGL_VENDOR_LIBRARY_FILENAMES:-}"
printf '__GLX_VENDOR_LIBRARY_NAME=%s\n' "${__GLX_VENDOR_LIBRARY_NAME:-}"
printf 'LIBGL_ALWAYS_SOFTWARE=%s\n' "${LIBGL_ALWAYS_SOFTWARE:-}"
printf 'MESA_LOADER_DRIVER_OVERRIDE=%s\n' "${MESA_LOADER_DRIVER_OVERRIDE:-}"
printf 'YAWC_DISABLE_CLIENT_DMABUF=%s\n' "${YAWC_DISABLE_CLIENT_DMABUF:-}"
printf 'YAWC_ENABLE_DMABUF_PROBE=%s\n' "${YAWC_ENABLE_DMABUF_PROBE:-}"
printf 'YAWC_ENABLE_CLIENT_DMABUF=%s\n' "${YAWC_ENABLE_CLIENT_DMABUF:-}"

section "User"
id

section "DRI Devices"
if [ -d /dev/dri ]; then
  ls -l /dev/dri
else
  echo "/dev/dri is missing"
fi

section "GLVND EGL Vendors"
if ls /usr/share/glvnd/egl_vendor.d/*.json >/dev/null 2>&1; then
  for vendor in /usr/share/glvnd/egl_vendor.d/*.json; do
    printf '%s: ' "$vendor"
    tr '\n' ' ' < "$vendor"
    printf '\n'
  done
else
  echo "No EGL vendor files found in /usr/share/glvnd/egl_vendor.d"
fi

section "Vulkan ICDs"
if ls /usr/share/vulkan/icd.d/*.json /etc/vulkan/icd.d/*.json >/dev/null 2>&1; then
  for icd in /usr/share/vulkan/icd.d/*.json /etc/vulkan/icd.d/*.json; do
    [ -e "$icd" ] || continue
    echo "$icd"
  done
else
  echo "No Vulkan ICD files found"
fi

section "Wayland EGL"
if command -v eglinfo >/dev/null 2>&1; then
  egl_output="$(mktemp)"
  EGL_PLATFORM="${EGL_PLATFORM:-wayland}" eglinfo -p wayland > "$egl_output" 2>&1 || true
  if ! grep -Ei 'EGL vendor|EGL version|EGL client APIs|EGL driver|OpenGL vendor|OpenGL renderer|llvmpipe|softpipe|swrast|nvidia|mesa|error|failed' "$egl_output"; then
    sed -n '1,80p' "$egl_output"
  fi
  rm -f "$egl_output"
else
  echo "eglinfo is not installed; install mesa-utils-extra for Wayland EGL diagnostics"
fi

section "OBS Check Command"
cat <<'EOF'
Run this from inside YAWC to see the renderer OBS actually chose:
  obs --verbose 2>&1 | grep -Ei 'OpenGL|adapter|renderer|EGL|llvmpipe|softpipe|swrast|nvidia|mesa|failed|error'

If this command is installed as yawc-debug-gpu-env, you can run it from any
directory. From the repository, the same script is available at:
  ./scripts/debug-gpu-env.sh

If OBS still says llvmpipe after relogin, try the NVIDIA override once from
the repository:
  YAWC_GPU_VENDOR=nvidia ./scripts/dev-update-session.sh
Then relogin to YAWC from SDDM and run:
  yawc-debug-gpu-env

YAWC advertises linux-dmabuf in normal standalone sessions. If a driver-specific
client buffer path needs to be bypassed temporarily, install the session with:
  YAWC_DISABLE_CLIENT_DMABUF=1 ./scripts/dev-update-session.sh

The diagnostic probe session is opt-in:
  YAWC_INSTALL_DMABUF_PROBE=1 ./scripts/dev-update-session.sh
EOF
