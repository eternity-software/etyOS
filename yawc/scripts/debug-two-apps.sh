#!/bin/sh
set -eu

FIREFOX_PROFILE_DIR="$(mktemp -d /tmp/yawc-firefox-profile.XXXXXX)"

nohup env MOZ_ENABLE_WAYLAND=1 firefox --new-instance --profile "$FIREFOX_PROFILE_DIR" >/tmp/yawc-firefox.log 2>&1 &
sleep 1
exec env QT_QPA_PLATFORM=wayland konsole --separate
