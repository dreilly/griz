#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
if [ ! -f vendor/ghostty-vt-1.0.0.xcframework/ghostty-vt.xcframework/macos-arm64_x86_64/libghostty-vt.a ]; then
    ./scripts/setup-ghostty.sh
fi
cargo build --workspace
exec ./target/debug/griz "$@"
