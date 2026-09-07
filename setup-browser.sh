#!/bin/bash
# Setup Native Messaging Host manifest untuk browser berbasis Chromium.
#
# Script ini hanya pelengkap: aplikasi juga menulis manifest sendiri
# (src/native_host/setup.rs). Karena itu keduanya HARUS menghasilkan
# allowed_origins yang sama — kalau tidak, menjalankan script ini akan
# memutus extension unpacked/dev yang sudah di-register lewat aplikasi.
set -euo pipefail

HOST_NAME="com.fastdm.native"
APP_DIR="/opt/fast-dm"
NATIVE_PATH="$APP_DIR/fast-dm-native"

# Sama dengan Config::config_dir() di sisi Rust (dirs::config_dir()).
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"
REGISTRY="$CONFIG_DIR/fast-dm/extension_ids.json"

# ID extension packed — JANGAN pakai wildcard "chrome-extension://*/*"
# karena ekstensi apa pun di browser user bisa memanggil native host (security).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EXT_ID=""
for candidate in "$SCRIPT_DIR/EXT_ID" "$PWD/EXT_ID"; do
  if [ -f "$candidate" ]; then
    EXT_ID="$(tr -d '[:space:]' < "$candidate")"
    if [ -n "$EXT_ID" ]; then
      break
    fi
  fi
done

# v2.9.3: dulu EXT_ID kosong tetap ditulis → "chrome-extension:///" dan native
# messaging mati tanpa pesan apa pun. Lebih baik gagal keras di sini.
if ! printf '%s' "$EXT_ID" | grep -qE '^[a-p]{32}$'; then
  echo "✗ EXT_ID tidak ditemukan / tidak valid." >&2
  echo "  Jalankan script ini dari folder repo (file EXT_ID harus ada di sebelahnya)." >&2
  exit 1
fi

# v2.9.3: gabungkan ID extension unpacked yang pernah di-register aplikasi
# (extension_ids.json) supaya script ini tidak menimpa/menghapus origin-nya.
IDS="$EXT_ID"
if [ -f "$REGISTRY" ]; then
  while read -r id; do
    [ -n "$id" ] || continue
    case " $IDS " in
      *" $id "*) continue ;;
    esac
    IDS="$IDS $id"
  done < <(grep -oE '[a-p]{32}' "$REGISTRY" || true)
fi

ORIGINS=""
for id in $IDS; do
  if [ -n "$ORIGINS" ]; then
    ORIGINS="$ORIGINS,
"
  fi
  ORIGINS="$ORIGINS    \"chrome-extension://$id/\""
done

HOST_JSON=$(cat <<JSON
{
  "name": "$HOST_NAME",
  "description": "Fast Download Manager Native Host",
  "path": "$NATIVE_PATH",
  "type": "stdio",
  "allowed_origins": [
$ORIGINS
  ]
}
JSON
)

WRITTEN=0
SEEN=""

# Tulis manifest hanya kalau profil browser-nya benar-benar ada (parent dir).
# Sama seperti M8 di sisi Rust: `mkdir -p` buta bikin ±13 folder sampah di
# ~/.config walau user cuma punya satu browser.
write_manifest() {
  local dir="$1"
  case " $SEEN " in
    *" $dir "*) return 1 ;;
  esac
  SEEN="$SEEN $dir"

  [ -d "$(dirname "$dir")" ] || return 1
  mkdir -p "$dir"
  printf '%s\n' "$HOST_JSON" > "$dir/$HOST_NAME.json"
  echo "✓ $dir/$HOST_NAME.json"
}

echo "Setting up Native Messaging Hosts..."

for browser in \
  google-chrome \
  chromium \
  thorium \
  BraveSoftware/Brave-Browser \
  vivaldi \
  opera \
  com.operasoftware.Opera \
  microsoft-edge \
  ungoogled-chromium \
  yandex-browser \
  sidekick \
  helium \
  net.imput.helium
do
  if write_manifest "$CONFIG_DIR/$browser/NativeMessagingHosts"; then
    WRITTEN=$((WRITTEN + 1))
  fi
done

# Profil Chromium lain: folder mana pun di config dir yang punya subfolder "Default".
for profile in "$CONFIG_DIR"/*/Default; do
  [ -d "$profile" ] || continue
  if write_manifest "$(dirname "$profile")/NativeMessagingHosts"; then
    WRITTEN=$((WRITTEN + 1))
  fi
done

for base in \
  "$HOME/.local/share/ice/profiles" \
  "$HOME/.local/share/helium/profiles"
do
  [ -d "$base" ] || continue
  for profile in "$base"/*; do
    [ -d "$profile" ] || continue
    if write_manifest "$profile/NativeMessagingHosts"; then
      WRITTEN=$((WRITTEN + 1))
    fi
  done
done

if [ "$WRITTEN" -eq 0 ]; then
  echo "✗ Tidak ada profil browser Chromium yang terdeteksi di $CONFIG_DIR." >&2
  echo "  Jalankan browser sekali dulu, lalu ulangi script ini." >&2
  exit 1
fi

if [ ! -x "$NATIVE_PATH" ]; then
  echo "! $NATIVE_PATH belum ada — install paket .deb dulu, atau jalankan aplikasi"
  echo "  sekali supaya manifest diarahkan ke binary hasil build."
fi

echo "Done ($WRITTEN manifest, $(printf '%s' "$IDS" | wc -w) origin). Restart browser dan reload extension."
