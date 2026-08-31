#!/bin/bash
set -e

HOST_NAME="com.fastdm.native"
APP_DIR="/opt/fast-dm"
NATIVE_PATH="$APP_DIR/fast-dm-native"

# ID extension packed — JANGAN pakai wildcard "chrome-extension://*/*"
# karena ekstensi apa pun di browser user bisa memanggil native host (security).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$SCRIPT_DIR/EXT_ID" ]; then
  EXT_ID="$(tr -d '[:space:]' < "$SCRIPT_DIR/EXT_ID")"
elif [ -f "$PWD/EXT_ID" ]; then
  EXT_ID="$(tr -d '[:space:]' < "$PWD/EXT_ID")"
fi

HOST_JSON=$(cat <<JSON
{
  "name": "$HOST_NAME",
  "description": "Fast Download Manager Native Host",
  "path": "$NATIVE_PATH",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://${EXT_ID}/"
  ]
}
JSON
)

write_manifest() {
  local dir="$1"
  mkdir -p "$dir"
  printf '%s\n' "$HOST_JSON" > "$dir/$HOST_NAME.json"
  echo "✓ $dir/$HOST_NAME.json"
}

echo "Setting up Native Messaging Hosts..."

for dir in \
  "$HOME/.config/google-chrome/NativeMessagingHosts" \
  "$HOME/.config/chromium/NativeMessagingHosts" \
  "$HOME/.config/thorium/NativeMessagingHosts" \
  "$HOME/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts" \
  "$HOME/.config/vivaldi/NativeMessagingHosts" \
  "$HOME/.config/opera/NativeMessagingHosts" \
  "$HOME/.config/com.operasoftware.Opera/NativeMessagingHosts" \
  "$HOME/.config/microsoft-edge/NativeMessagingHosts" \
  "$HOME/.config/ungoogled-chromium/NativeMessagingHosts" \
  "$HOME/.config/yandex-browser/NativeMessagingHosts" \
  "$HOME/.config/sidekick/NativeMessagingHosts" \
  "$HOME/.config/helium/NativeMessagingHosts"
do
  write_manifest "$dir"
done

for base in \
  "$HOME/.local/share/ice/profiles" \
  "$HOME/.local/share/helium/profiles"
do
  if [ -d "$base" ]; then
    for profile in "$base"/*; do
      [ -d "$profile" ] || continue
      write_manifest "$profile/NativeMessagingHosts"
    done
  fi
done

echo "Done. Restart browser and reload extension."
