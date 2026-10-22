#!/usr/bin/env sh
# Downloads the pinned mihomo release for bundling as a Tauri sidecar
# (`externalBin: ["binaries/mihomo"]`) and verifies its SHA-256.
# Optional: by default the launcher downloads the same pinned build on first
# use (crates/lc-vpn/src/binary.rs). Keep version and digests in sync with it.
#
# Usage: ./fetch-mihomo.sh [target-triple]   (default: x86_64-unknown-linux-gnu)
set -eu
VERSION=v1.19.33
TARGET=${1:-x86_64-unknown-linux-gnu}
case "$TARGET" in
  x86_64-unknown-linux-gnu)  NAME=mihomo-linux-amd64-compatible-$VERSION.gz; SHA=386508bfb1c72c43b6a0eda73708ecdce82546581b0ebcb16366c53757c622b6 ;;
  aarch64-unknown-linux-gnu) NAME=mihomo-linux-arm64-$VERSION.gz; SHA=ba408b317080ee265c46cb923a50018c1b82cbd708551cdd46a1e3e307396187 ;;
  *) echo "unsupported target $TARGET (use fetch-mihomo.ps1 for Windows)" >&2; exit 1 ;;
esac
DIR=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/$NAME" "https://github.com/MetaCubeX/mihomo/releases/download/$VERSION/$NAME"
echo "$SHA  $TMP/$NAME" | sha256sum -c -
gunzip -c "$TMP/$NAME" > "$DIR/mihomo-$TARGET"
chmod 755 "$DIR/mihomo-$TARGET"
echo "mihomo $VERSION for $TARGET ready in $DIR"
