#!/bin/sh
# Build the macOS bundle icon from the checked-in Terminal Gremlin artwork.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ICONSET=$(mktemp -d /tmp/griz-app-icon.XXXXXX)/Griz.iconset
mkdir -p "$ICONSET"
trap 'rm -rf "$(dirname "$ICONSET")"' EXIT HUP INT TERM
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$ROOT/assets/app-icon/terminal-gremlin.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$ROOT/assets/app-icon/terminal-gremlin.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$ROOT/assets/app-icon/Griz.icns"
