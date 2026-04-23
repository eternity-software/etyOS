#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="$ROOT_DIR/.tools/node/bin:$PATH"
unset ELECTRON_RUN_AS_NODE

cd "$ROOT_DIR/nirvana"
exec npm "$@"
