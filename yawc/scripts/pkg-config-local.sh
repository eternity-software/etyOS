#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SYSROOT_ROOT="$ROOT_DIR/.sysroot/root"
PKGCONF_BIN="$SYSROOT_ROOT/usr/bin/pkg-config"
PKGCONF_LIBDIR="$SYSROOT_ROOT/usr/lib/x86_64-linux-gnu"

if [ ! -x "$PKGCONF_BIN" ]; then
  echo "error: local pkg-config is missing, run ./scripts/bootstrap-standalone-deps.sh first" >&2
  exit 1
fi

export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}$PKGCONF_LIBDIR"
exec "$PKGCONF_BIN" "$@"
