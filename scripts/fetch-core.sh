#!/usr/bin/env bash
# Fetch the engine.
#
# xproxy does not vendor GPROXY: the crates in `gproxy-ref/` are the upstream
# repository at a pinned commit, referenced by path from the workspace
# manifest. This script puts that checkout in place, and `--update` moves the
# pin when you want a newer engine.
#
# The pin is a full SHA, not `main`: an engine that can be swapped without a
# diff is an engine whose behaviour cannot be reproduced.

set -euo pipefail

PIN="97b5ba7efa9b09e5a87d6769e9d6208dccfa1925"
REMOTE="https://github.com/LeenHawk/gproxy.git"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="$HERE/gproxy-ref"

if [[ "${1:-}" == "--update" ]]; then
  if [[ ! -d "$TARGET/.git" ]]; then
    echo "gproxy-ref is not a checkout; run this script without --update first" >&2
    exit 1
  fi
  git -C "$TARGET" fetch --depth 1 origin
  PIN="$(git -C "$TARGET" rev-parse FETCH_HEAD)"
  git -C "$TARGET" checkout --detach "$PIN"
  echo "updated gproxy-ref to $PIN"
  echo "now update PIN in this script and the table in README.md"
  exit 0
fi

if [[ -d "$TARGET/.git" ]]; then
  echo "gproxy-ref already exists; pass --update to move the pin"
  exit 0
fi

# A shallow fetch of one commit: the history is a hundred megabytes of
# release notes and this project needs none of it.
mkdir -p "$TARGET"
git -C "$TARGET" init --quiet
git -C "$TARGET" remote add origin "$REMOTE"
git -C "$TARGET" fetch --depth 1 origin "$PIN"
git -C "$TARGET" checkout --quiet --detach "$PIN"
echo "gproxy-ref is at $PIN"
