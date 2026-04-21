#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
echo "warning: run-tty-direct.sh is deprecated; use run-standalone-direct.sh" >&2
exec sh "$ROOT_DIR/scripts/run-standalone-direct.sh" "$@"
