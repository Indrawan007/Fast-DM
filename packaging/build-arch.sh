#!/bin/bash#!/bin/bash
# Build paket Arch Linux x86_64 untuk Hyprland/Wayland.
# Output: build/fast-dm-<versi>-<pkgrel>-x86_64.pkg.tar.zst
# Jalankan di Arch Linux (atau container `archlinux`) dengan:
#   sudo pacman -S --needed base-devel rust gtk4 aria2 yt-dlp ffmpeg xdg-utils wl-clipboard
#
# Kenapa tidak langsung `makepkg` di folder packaging/?
#   1. PKGBUILD memakai placeholder @VERSION@/@PKGREL@ yang diisi dari
#      Cargo.toml (satu sumber versi, AGENTS.md §5),
#   2. PKGBUILD mengharapkan source tarball fast-dm-<versi>.tar.gz, jadi
#      working tree harus dibungkus dulu,
#   3. artefak paket harus dikumpulkan ke direktori build/.
set -e
cd "$(dirname "$0")/.."
ROOT="$PWD"

VER=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\(.*\)".*/\1/p;}' Cargo.toml | head -1)
PKGREL="${PKGREL:-1}"

if [ -z "$VER" ]; then
  echo "✗ versi tidak terbaca dari Cargo.toml" >&2
  exit 1
fi

if ! command -v makepkg >/dev/null 2>&1; then
  echo "✗ makepkg tidak ditemukan — skrip ini membutuhkan Arch Linux." >&2
  exit 1
fi

# makepkg menolak jalan sebagai root (dan memang seharusnya).
if [ "$(id -u)" -eq 0 ]; then
  echo "✗ makepkg tidak boleh dijalankan sebagai root." >&2
  echo "  Jalankan sebagai user biasa; di CI lihat job 'arch' pada ci.yml." >&2
  exit 1
fi

echo "==> Building fast-dm $VER-$PKGREL (Arch Linux)"

STAGE="$ROOT/build/arch"
rm -rf "$STAGE"
mkdir -p "$STAGE" "$ROOT/build"

# 1. Source tarball dengan nama yang diharapkan PKGBUILD. Folder di dalamnya
#    di-rename ke fast-dm-<versi> agar $srcdir cocok. Artefak build dikecualikan
#    — termasuk build/ sendiri, tempat staging ini berada (anti rekursi).
echo "==> Packing source tree"
tar -czf "$STAGE/fast-dm-$VER.tar.gz" \
    --exclude=./.git \
    --exclude=./target \
    --exclude=./build \
    --exclude=./node_modules \
    --transform "s,^\./,fast-dm-$VER/," \
    -C "$ROOT" .

# 2. PKGBUILD dengan versi terisi.
sed "s/@VERSION@/$VER/; s/@PKGREL@/$PKGREL/" packaging/PKGBUILD > "$STAGE/PKGBUILD"

# 3. Build paket.
cd "$STAGE"
makepkg -f --noconfirm

# 4. Kumpulkan artefak ke build/.
cp "$STAGE"/*.pkg.tar.* "$ROOT/build/"
# Jangan parse output `ls`: glob Bash mempertahankan nama dengan spasi dan
# `-f` membedakan glob yang tidak cocok dari artefak sungguhan.
PKGFILES=("$ROOT"/build/*"$VER-$PKGREL"*.pkg.tar.*)
PKGFILE="${PKGFILES[0]}"

if [ -f "$PKGFILE" ]; then
  echo "✓ $PKGFILE"
  echo "  Install: sudo pacman -U $(basename "$PKGFILE")"
else
  echo "✗ paket tidak ditemukan di build/" >&2
  exit 1
fi
