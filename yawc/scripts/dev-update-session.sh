#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

"$ROOT_DIR/scripts/install-session.sh" "$@"

cat <<'EOF'

YAWC was rebuilt and the login-session binary was updated.

To restart into the new build from inside YAWC:
  1. Save anything important in clients.
  2. Press Ctrl+Alt+Backspace or Ctrl+Alt+Esc.
  3. Select the YAWC session again on the login screen.

EOF
