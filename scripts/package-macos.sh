#!/bin/sh
# Developer-side entry point; testers need only the resulting app.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
exec python3 "$ROOT/scripts/package_macos.py" "$@"
