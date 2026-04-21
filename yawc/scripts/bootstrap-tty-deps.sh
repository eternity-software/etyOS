#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
echo "warning: bootstrap-tty-deps.sh is deprecated; use bootstrap-standalone-deps.sh" >&2
exec sh "$ROOT_DIR/scripts/bootstrap-standalone-deps.sh" "$@"
