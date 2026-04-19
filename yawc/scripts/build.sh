#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
LOCAL_LIB_DIR="$ROOT_DIR/.local-lib"

mkdir -p "$LOCAL_LIB_DIR"
ln -sf /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0 "$LOCAL_LIB_DIR/libxkbcommon.so"

cd "$ROOT_DIR"
exec env RUSTFLAGS="-L native=$LOCAL_LIB_DIR" cargo build "$@"
