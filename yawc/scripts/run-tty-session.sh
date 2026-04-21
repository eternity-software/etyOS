#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
echo "warning: run-tty-session.sh is deprecated; use run-standalone-session.sh" >&2
exec sh "$ROOT_DIR/scripts/run-standalone-session.sh" "$@"
