#!/usr/bin/env bash
# Install chops desktop app on macOS from GitHub releases.
# Usage: ./scripts/install-mac.sh [stable|dev]
set -euo pipefail

REPO="thompsonson/chops"
CHANNEL="${1:-dev}"
APP_NAME="chops.app"
APP_DIR="/Applications"

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

gh release download "$TAG" --repo "$REPO" --pattern '*.dmg' --dir "$TMPDIR"
DMG=$(ls "$TMPDIR"/*.dmg 2>/dev/null | head -1)

if [ -z "$DMG" ]; then
  echo "Error: no DMG found in release $TAG"
  exit 1
fi

echo "Installing from $(basename "$DMG")..."

# Mount, copy, unmount
MOUNT=$(hdiutil attach "$DMG" -nobrowse -quiet | grep '/Volumes' | cut -f3-)
if [ -z "$MOUNT" ]; then
  echo "Error: failed to mount DMG"
  exit 1
fi

# Kill running instance if any
osascript -e 'quit app "chops"' 2>/dev/null || true
sleep 1

# Copy to /Applications
if [ -d "$APP_DIR/$APP_NAME" ]; then
  echo "Replacing existing $APP_NAME..."
  rm -rf "$APP_DIR/$APP_NAME"
fi
cp -R "$MOUNT/$APP_NAME" "$APP_DIR/"

hdiutil detach "$MOUNT" -quiet

echo "Installed to $APP_DIR/$APP_NAME"
echo "Run: open -a chops"
