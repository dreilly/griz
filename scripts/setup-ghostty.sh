#!/bin/sh
set -eu

VERSION=1.0.0
SHA256=2c32b7409b2b6c2ef6cfd606526de357762ad458bdc27134ad074e86506a8bc2
ARCHIVE="ghostty-vt-${VERSION}.xcframework.zip"
URL="https://release.files.ghostty.org/libghostty-vt/${VERSION}/${ARCHIVE}"

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
VENDOR="$ROOT/vendor"
ZIP="$VENDOR/$ARCHIVE"
DEST="$VENDOR/ghostty-vt-${VERSION}.xcframework"
FRAMEWORK="$DEST/ghostty-vt.xcframework"
MACOS_SLICE="$FRAMEWORK/macos-arm64_x86_64"

if [ "$(uname -s)" != Darwin ]; then
    echo "setup-ghostty.sh: libghostty-vt 1.0.0 prebuilt setup currently supports macOS only" >&2
    exit 1
fi

for command in curl unzip shasum lipo; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "setup-ghostty.sh: required command not found: $command" >&2
        exit 1
    fi
done

mkdir -p "$VENDOR"

verify_archive() {
    actual=$(shasum -a 256 "$ZIP" | awk '{print $1}')
    [ "$actual" = "$SHA256" ]
}

if [ -f "$ZIP" ] && ! verify_archive; then
    echo "setup-ghostty.sh: removing downloaded archive with an invalid checksum" >&2
    rm -f "$ZIP"
fi

if [ ! -f "$ZIP" ]; then
    tmp="$ZIP.tmp.$$"
    trap 'rm -f "$tmp"' EXIT HUP INT TERM
    echo "Downloading libghostty-vt $VERSION..."
    curl -fL --retry 3 -o "$tmp" "$URL"
    mv "$tmp" "$ZIP"
    trap - EXIT HUP INT TERM
fi

if ! verify_archive; then
    echo "setup-ghostty.sh: checksum verification failed for $ZIP" >&2
    exit 1
fi

tmp_dest="$DEST.tmp.$$"
rm -rf "$tmp_dest"
mkdir -p "$tmp_dest"
trap 'rm -rf "$tmp_dest"' EXIT HUP INT TERM
unzip -q "$ZIP" -d "$tmp_dest"

if [ ! -f "$tmp_dest/ghostty-vt.xcframework/macos-arm64_x86_64/libghostty-vt.a" ] || \
   [ ! -f "$tmp_dest/ghostty-vt.xcframework/macos-arm64_x86_64/Headers/ghostty/vt.h" ]; then
    echo "setup-ghostty.sh: downloaded XCFramework does not contain the expected macOS library and headers" >&2
    exit 1
fi

archs=$(lipo -archs "$tmp_dest/ghostty-vt.xcframework/macos-arm64_x86_64/libghostty-vt.a")
case " $archs " in
    *" arm64 "*) ;;
    *)
        echo "setup-ghostty.sh: macOS static library is missing the arm64 architecture (found: $archs)" >&2
        exit 1
        ;;
esac

rm -rf "$DEST"
mv "$tmp_dest" "$DEST"
trap - EXIT HUP INT TERM

echo "Installed libghostty-vt $VERSION at $MACOS_SLICE"
echo "Architectures: $archs"
