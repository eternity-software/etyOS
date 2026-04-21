#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
echo "warning: launch-tty-binary.sh is deprecated; use launch-standalone-binary.sh" >&2
exec sh "$ROOT_DIR/scripts/launch-standalone-binary.sh" "$@"
