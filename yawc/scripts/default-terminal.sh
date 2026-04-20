#!/bin/sh
set -eu

export PATH="${PATH:-/usr/local/bin:/usr/bin:/bin}"

for candidate in \
  foot \
  weston-terminal \
  kgx \
  alacritty \
  wezterm \
  kitty \
  konsole \
  gnome-terminal \
  xfce4-terminal \
  qterminal
do
  if command -v "$candidate" >/dev/null 2>&1; then
    printf '%s\n' "$candidate"
    exit 0
  fi
done

echo "error: no supported terminal emulator found in PATH" >&2
exit 1
