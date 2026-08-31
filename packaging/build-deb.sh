#!/bin/bash
# Build .deb installer Fast-DM → output: build/fast-dm_<versi>_amd64.deb
set -e
cd "$(dirname "$0")/.."

VER=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\(.*\)".*/\1/p;}' Cargo.toml | head -1)
echo "==> Building fast-dm $VER"
cargo build --release

PKG="build/fast-dm_${VER}_amd64"
rm -rf "$PKG"
mkdir -p "$PKG/DEBIAN" \
         "$PKG/opt/fast-dm" \
         "$PKG/usr/bin" \
         "$PKG/usr/share/applications" \
         "$PKG/usr/share/icons/hicolor/128x128/apps"

# Binary + wrapper native messaging host
cp target/release/fast-dm "$PKG/opt/fast-dm/fast-dm"
cat > "$PKG/opt/fast-dm/fast-dm-native" <<'EOF'
#!/bin/sh
exec /opt/fast-dm/fast-dm --native
EOF
chmod 755 "$PKG/opt/fast-dm/fast-dm" "$PKG/opt/fast-dm/fast-dm-native"

# Symlink agar bisa dipanggil dari terminal
ln -sf /opt/fast-dm/fast-dm "$PKG/usr/bin/fast-dm"

# control + postinst
sed "s/@VERSION@/$VER/" packaging/control > "$PKG/DEBIAN/control"
cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
EOF
chmod 755 "$PKG/DEBIAN/postinst"

# Desktop entry + icon
cp packaging/fast-dm.desktop "$PKG/usr/share/applications/fast-dm.desktop"
cp extension/icons/icon128.png \
   "$PKG/usr/share/icons/hicolor/128x128/apps/io.github.fastdm.FastDownloadManager.png"

dpkg-deb --build "$PKG"
echo "✓ $PKG.deb"
