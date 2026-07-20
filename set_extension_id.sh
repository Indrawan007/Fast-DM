#!/bin/bash
#
# Update Extension ID di Native Messaging Host manifest
#
# Usage: bash set_extension_id.sh <EXTENSION_ID>
#

if [ -z "$1" ]; then
    echo "Usage: $0 <CHROME_EXTENSION_ID>"
    echo ""
    echo "Find your Extension ID at chrome://extensions"
    echo "Example: $0 abcdefghijklmnopqrstuvwxyz123456"
    exit 1
fi

EXT_ID="$1"
NATIVE_HOST_NAME="com.fastdm.native"

echo "Setting Extension ID to: $EXT_ID"

# Update Chrome
CHROME_MANIFEST="$HOME/.config/google-chrome/NativeMessagingHosts/${NATIVE_HOST_NAME}.json"
if [ -f "$CHROME_MANIFEST" ]; then
    sed -i "s|chrome-extension://[^/]*/|chrome-extension://${EXT_ID}/|g" "$CHROME_MANIFEST"
    # Also replace placeholder
    sed -i "s|EXTENSION_ID_PLACEHOLDER|${EXT_ID}|g" "$CHROME_MANIFEST"
    echo "✓ Updated: $CHROME_MANIFEST"
fi

# Update Chromium
CHROMIUM_MANIFEST="$HOME/.config/chromium/NativeMessagingHosts/${NATIVE_HOST_NAME}.json"
if [ -f "$CHROMIUM_MANIFEST" ]; then
    sed -i "s|chrome-extension://[^/]*/|chrome-extension://${EXT_ID}/|g" "$CHROMIUM_MANIFEST"
    sed -i "s|EXTENSION_ID_PLACEHOLDER|${EXT_ID}|g" "$CHROMIUM_MANIFEST"
    echo "✓ Updated: $CHROMIUM_MANIFEST"
fi

echo ""
echo "Done! Restart Chrome for changes to take effect."
