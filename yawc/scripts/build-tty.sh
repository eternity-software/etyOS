#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
echo "warning: build-tty.sh is deprecated; use build-standalone.sh" >&2
exec sh "$ROOT_DIR/scripts/build-standalone.sh" "$@"
