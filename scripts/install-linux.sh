#!/usr/bin/env bash
# Install chops desktop app on Linux from GitHub releases.
# Usage: ./scripts/install-linux.sh [stable|dev]
set -euo pipefail

REPO="thompsonson/chops"
CHANNEL="${1:-dev}"

if [ "$CHANNEL" = "stable" ]; then
  TAG=$(gh release list --repo "$REPO" --limit 10 --json tagName,isPrerelease \
    --jq '[.[] | select(.isPrerelease == false)][0].tagName // empty')
  if [ -z "$TAG" ]; then
    echo "No stable release found. Use: $0 dev"
    exit 1
  fi
else
  TAG="dev-desktop"
fi

echo "Downloading chops ($CHANNEL) from release: $TAG"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

gh release download "$TAG" --repo "$REPO" --pattern '*.AppImage' --dir "$TMPDIR"
APPIMAGE=$(ls "$TMPDIR"/*.AppImage 2>/dev/null | head -1)

if [ -z "$APPIMAGE" ]; then
  echo "Error: no AppImage found in release $TAG"
  exit 1
fi

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"
DEST="$INSTALL_DIR/chops.AppImage"

# Stop running instance if any
pkill -f 'chops.AppImage' 2>/dev/null || true
sleep 1

cp "$APPIMAGE" "$DEST"
chmod +x "$DEST"

echo "Installed to $DEST"
echo "Run: chops.AppImage"
