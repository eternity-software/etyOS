#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="$ROOT_DIR/.tools/node/bin:$PATH"
unset ELECTRON_RUN_AS_NODE

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
  echo "Nirvana requires Wayland. WAYLAND_DISPLAY is not set." >&2
  exit 1
fi

export XDG_SESSION_TYPE=wayland
unset DISPLAY

cd "$ROOT_DIR/nirvana"
exec npm run dev
