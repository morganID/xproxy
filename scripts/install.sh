#!/bin/sh
# Install the xproxy command line from GitHub. No clone, no compiler.
#
#   curl -fsSL https://raw.githubusercontent.com/morganID/xproxy/main/scripts/install.sh | sh
#
# Downloads the latest release for this architecture and puts the binary in the
# first writable directory on the PATH — Homebrew's bin on Apple Silicon, then
# /usr/local/bin, then ~/.local/bin.
set -eu

REPO=${XPROXY_REPO:-morganID/xproxy}
ARCH=$(uname -m)
ASSET="xproxy-macos-$ARCH.tar.gz"

echo "→ looking up the latest release of $REPO"
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n "s/.*\"browser_download_url\": \"\\([^\"]*$ASSET\\)\".*/\\1/p" | head -1)
if [ -z "$URL" ]; then
    echo "no release asset for $ARCH in $REPO" >&2
    echo "build it yourself:" >&2
    echo "  git clone https://github.com/$REPO && cd xproxy" >&2
    echo "  ./scripts/fetch-core.sh && cargo build --release -p xproxy" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
echo "→ downloading $ASSET"
curl -fsSL "$URL" -o "$TMP/$ASSET"
tar -xzf "$TMP/$ASSET" -C "$TMP"

DEST=""
for dir in /opt/homebrew/bin /usr/local/bin "$HOME/.local/bin"; do
    if [ -d "$dir" ] && [ -w "$dir" ]; then
        DEST="$dir"
        break
    fi
done
if [ -z "$DEST" ]; then
    mkdir -p "$HOME/.local/bin"
    DEST="$HOME/.local/bin"
fi

install -m 755 "$TMP/xproxy" "$DEST/xproxy"
echo "installed  $DEST/xproxy"
"$DEST/xproxy" --version 2>/dev/null || true
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "note       $DEST is not on your PATH" ;;
esac
