#!/bin/bash
#
# Fast Download Manager - Uninstaller
#

set -e

NATIVE_HOST_NAME="com.fastdm.native"
APP_NAME="fast-dm"

echo "Uninstalling Fast Download Manager..."

# Remove native messaging host
rm -f "$HOME/.config/google-chrome/NativeMessagingHosts/${NATIVE_HOST_NAME}.json"
rm -f "$HOME/.config/chromium/NativeMessagingHosts/${NATIVE_HOST_NAME}.json"
echo "✓ Native messaging host removed"

# Remove desktop entry
rm -f "$HOME/.local/share/applications/${APP_NAME}.desktop"
echo "✓ Desktop entry removed"

# Remove config
rm -rf "$HOME/.config/fast-dm"
echo "✓ Config removed"

# Remove socket
rm -f /tmp/fast-dm.sock
echo "✓ Socket removed"

echo ""
echo "Uninstall complete!"
echo "Note: The extension directory and aria2 are NOT removed."
echo "Remove the extension manually from chrome://extensions"
