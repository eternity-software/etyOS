#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SYSROOT_DIR="$ROOT_DIR/.sysroot"
DEB_DIR="$SYSROOT_DIR/debs"
ROOTFS_DIR="$SYSROOT_DIR/root"

if ! command -v apt-get >/dev/null 2>&1; then
  echo "error: apt-get is required to bootstrap local tty-udev dependencies" >&2
  exit 1
fi

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "error: dpkg-deb is required to unpack local tty-udev dependencies" >&2
  exit 1
fi

mkdir -p "$DEB_DIR" "$ROOTFS_DIR"

cd "$DEB_DIR"
apt-get download \
  pkg-config pkgconf pkgconf-bin libpkgconf3 \
  libinput-dev libgbm-dev libgbm1 libdrm-dev libseat-dev libudev-dev libudev1 seatd \
  libmtdev-dev libevdev-dev libwacom-dev libpciaccess-dev libcap-dev \
  libseat1 libinput10

for deb in ./*.deb; do
  dpkg-deb -x "$deb" "$ROOTFS_DIR"
done

echo "bootstrapped local tty-udev sysroot in $SYSROOT_DIR"
