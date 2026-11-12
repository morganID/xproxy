#!/usr/bin/env bash
# Put BoringSSL where `btls-sys` looks for it, without cloning its history.
#
# `btls-sys` — the TLS backend one family of channels uses for its browser
# fingerprint — carries BoringSSL as a git submodule and runs
#
#     git submodule update --init --recursive
#
# from its build script when `deps/boringssl/CMakeLists.txt` is absent. That
# fetches the submodule's entire history: hundreds of megabytes, and over a
# slow link hours of nothing. The build needs one tree at one commit, so this
# script downloads the archive of exactly that commit and drops it in place.
# The build script then finds the file it checks for, skips the fetch, and
# copies the tree into `OUT_DIR` before applying its patches — nothing here is
# modified.
#
# Run it once after `cargo fetch`, and only if the build stalls on that step.

set -euo pipefail

BORINGSSL_COMMIT="f1f2556a5dfa59e147d9d47279cc3f7f8a18b433"
BORINGSSL_REMOTE="https://github.com/google/boringssl.git"
ARCHIVE="https://github.com/google/boringssl/archive/${BORINGSSL_COMMIT}.tar.gz"
# The same archive through a mirror, for networks where github.com crawls.
ARCHIVE_MIRROR="https://gh-proxy.com/https://github.com/google/boringssl/archive/${BORINGSSL_COMMIT}.tar.gz"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/xproxy"

shopt -s nullglob
checkouts=("$HOME"/.cargo/git/checkouts/btls-*/*)
if [[ ${#checkouts[@]} -eq 0 ]]; then
  echo "no btls checkout under ~/.cargo/git/checkouts — run 'cargo fetch' first" >&2
  exit 1
fi

fetch() {
  local tree="$CACHE/boringssl"
  if [[ -f "$tree/CMakeLists.txt" ]]; then
    return
  fi
  mkdir -p "$CACHE"
  rm -rf "$tree" "$CACHE/boringssl.tar.gz"
  for url in "$ARCHIVE" "$ARCHIVE_MIRROR"; do
    echo "downloading $url"
    if curl -fsSL --retry 2 --max-time 1800 -o "$CACHE/boringssl.tar.gz" "$url"; then
      break
    fi
    echo "that one did not work; trying the next"
  done
  [[ -s "$CACHE/boringssl.tar.gz" ]] || { echo "could not download BoringSSL" >&2; exit 1; }
  mkdir -p "$tree"
  # GitHub archives nest the tree in `boringssl-<sha>/`; strip one component.
  tar -xzf "$CACHE/boringssl.tar.gz" -C "$tree" --strip-components=1
  rm -f "$CACHE/boringssl.tar.gz"
  [[ -f "$tree/CMakeLists.txt" ]] || { echo "the archive had no CMakeLists.txt" >&2; exit 1; }
}

fetch

for checkout in "${checkouts[@]}"; do
  target="$checkout/btls-sys/deps/boringssl"
  [[ -d "$checkout/btls-sys" ]] || continue
  if [[ -f "$target/CMakeLists.txt" ]]; then
    echo "already in place: $target"
    continue
  fi
  mkdir -p "$target"
  # `cp -R src/. dst/` copies the contents; `cp -R src dst` would nest them.
  cp -R "$CACHE/boringssl/." "$target/"
  echo "placed BoringSSL in $target"
done

echo "note: a 'cargo clean' of the git cache re-extracts the checkout; re-run this then."
