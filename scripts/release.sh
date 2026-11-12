#!/bin/sh
# Cut a release of the command line: build, pack, publish.
#
#   ./scripts/release.sh            # tag v<version from Cargo.toml>
#   ./scripts/release.sh v0.2.0
#
# The asset is `xproxy-macos-<arch>.tar.gz`, which scripts/install.sh
# downloads. The engine must already be checked out: run scripts/fetch-core.sh
# first, or the build fails looking for gproxy-ref/.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TAG=${1:-v$VERSION}
ARCH=$(uname -m)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}

if [ ! -d "$ROOT/gproxy-ref" ]; then
    echo "the engine is not checked out; run ./scripts/fetch-core.sh first" >&2
    exit 1
fi

echo "→ building xproxy for $ARCH"
cargo build --release -p xproxy

mkdir -p "$ROOT/dist"
ASSET="$ROOT/dist/xproxy-macos-$ARCH.tar.gz"
echo "→ packing $(basename "$ASSET")"
tar -czf "$ASSET" -C "$TARGET/release" xproxy
ls -lh "$ASSET" | awk '{print "  " $5, $9}'

echo "→ publishing $TAG"
NOTES="Built for $ARCH from $(git rev-parse --short HEAD 2>/dev/null || echo 'a working tree').

Install:

    curl -fsSL https://raw.githubusercontent.com/morganID/xproxy/main/scripts/install.sh | sh

Opus, the menu bar app that runs this gateway, installs separately:

    curl -fsSL https://raw.githubusercontent.com/morganID/opus/main/scripts/install.sh | sh"
gh release create "$TAG" "$ASSET" \
    --title "xproxy $VERSION" \
    --notes "$NOTES"

echo
echo "done       https://github.com/morganID/xproxy/releases/tag/$TAG"
