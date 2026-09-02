#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

APP_NAME="fast-dm"
APP_VERSION="2.1.3"
ARCH=$(dpkg --print-architecture 2>/dev/null || echo "amd64")

echo "Building Fast DM v${APP_VERSION} (Rust)"

# Install Rust build deps
if ! command -v cargo &>/dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

# Install GTK4 dev libs
sudo apt-get install -y -qq \
    libgtk-4-dev libglib2.0-dev \
    aria2 curl yt-dlp 2>/dev/null || true

# Build release
echo "Compiling (this takes 2-5 minutes first time)..."
cargo build --release 2>&1 | tail -5

BINARY="target/release/fast-dm"
if [ ! -f "$BINARY" ]; then
    echo "Build failed!"
    exit 1
fi

BINARY_SIZE=$(du -sh "$BINARY" | awk '{print $1}')
echo "Binary: $BINARY ($BINARY_SIZE)"

# Build .deb
BUILD_DIR="build"
DIST_DIR="dist"
DEB_ROOT="${BUILD_DIR}/${APP_NAME}_${APP_VERSION}_${ARCH}"

rm -rf "$BUILD_DIR" "$DIST_DIR"
mkdir -p "$DIST_DIR"
mkdir -p "${DEB_ROOT}/DEBIAN"
mkdir -p "${DEB_ROOT}/opt/${APP_NAME}/extension/icons"
mkdir -p "${DEB_ROOT}/usr/bin"
mkdir -p "${DEB_ROOT}/usr/share/applications"
mkdir -p "${DEB_ROOT}/usr/share/icons/hicolor/128x128/apps"

# Copy binary
cp "$BINARY" "${DEB_ROOT}/opt/${APP_NAME}/fast-dm"
chmod 755 "${DEB_ROOT}/opt/${APP_NAME}/fast-dm"

# Native host wrapper
cat > "${DEB_ROOT}/opt/${APP_NAME}/fast-dm-native" << 'EOF'
#!/bin/bash
exec /opt/fast-dm/fast-dm --native
EOF
chmod 755 "${DEB_ROOT}/opt/${APP_NAME}/fast-dm-native"

# Extension
cp -r extension/* "${DEB_ROOT}/opt/${APP_NAME}/extension/" 2>/dev/null || true

# Icons — pakai nama unik agar tidak collide dengan icon lain
for size in 16 32 48 128; do
    src="extension/icons/icon${size}.png"
    if [ -f "$src" ]; then
        # Copy ke extension folder (untuk browser)
        cp "$src" "${DEB_ROOT}/opt/${APP_NAME}/extension/icons/"

        # Copy ke system icons dengan nama UNIK (bukan fast-dm.png!)
        mkdir -p "${DEB_ROOT}/usr/share/icons/hicolor/${size}x${size}/apps"
        cp "$src" "${DEB_ROOT}/usr/share/icons/hicolor/${size}x${size}/apps/io.github.fastdm.FastDownloadManager.png"
    fi
done

# App icon di /opt (untuk backup)
if [ -f "extension/icons/icon128.png" ]; then
    cp "extension/icons/icon128.png" "${DEB_ROOT}/opt/${APP_NAME}/fast-dm-icon.png"
fi

# Tambah di build.sh setelah copy extension files

# Setup script
if [ -f "${SCRIPT_DIR}/setup-browser.sh" ]; then
    cp "${SCRIPT_DIR}/setup-browser.sh" "${DEB_ROOT}/opt/${APP_NAME}/setup-browser.sh"
else
    cat > "${DEB_ROOT}/opt/${APP_NAME}/setup-browser.sh" << 'EOF'
#!/bin/bash
echo "setup-browser.sh not found in source tree."
exit 1
EOF
fi
chmod 755 "${DEB_ROOT}/opt/${APP_NAME}/setup-browser.sh"
ln -sf "/opt/${APP_NAME}/setup-browser.sh" "${DEB_ROOT}/usr/bin/fast-dm-setup"

# Symlink
ln -sf "/opt/${APP_NAME}/fast-dm" "${DEB_ROOT}/usr/bin/fast-dm"

# Desktop entry — pakai application_id yang unik
cat > "${DEB_ROOT}/usr/share/applications/${APP_NAME}.desktop" << DESKTOP
[Desktop Entry]
Version=1.0
Type=Application
Name=Fast Download Manager
GenericName=Download Manager
Comment=High-speed download manager with browser integration
Exec=fast-dm
Icon=io.github.fastdm.FastDownloadManager
Terminal=false
Categories=Network;FileTransfer;GTK;
Keywords=download;manager;video;aria2;fast;
StartupNotify=true
StartupWMClass=io.github.fastdm.FastDownloadManager
DESKTOP

# DEBIAN/control
INSTALLED_SIZE=$(du -sk "${DEB_ROOT}" | awk '{print $1}')
cat > "${DEB_ROOT}/DEBIAN/control" << CTRL
Package: ${APP_NAME}
Version: ${APP_VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Installed-Size: ${INSTALLED_SIZE}
Depends: aria2, curl, libgtk-4-1
Recommends: yt-dlp
Maintainer: FastDM <fastdm@local>
Description: Fast Download Manager
 High-speed download manager with browser integration.
CTRL

# postinst
cat > "${DEB_ROOT}/DEBIAN/postinst" << 'POST'
#!/bin/bash
set -e

APP_DIR="/opt/fast-dm"
NATIVE_SCRIPT="${APP_DIR}/fast-dm-native"

# 1. Buat native host wrapper
cat > "${NATIVE_SCRIPT}" << 'NEOF'
#!/bin/bash
exec /opt/fast-dm/fast-dm --native
NEOF
chmod 755 "${NATIVE_SCRIPT}"

# 2. Setup manifest untuk SEMUA user yang punya home
NMH_JSON='{
  "name": "com.fastdm.native",
  "description": "Fast Download Manager Native Host",
  "path": "/opt/fast-dm/fast-dm-native",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://*/*"
  ]
}'

BROWSERS=(
    "google-chrome"
    "chromium"
    "thorium"
    "BraveSoftware/Brave-Browser"
    "vivaldi"
    "opera"
    "com.operasoftware.Opera"
    "microsoft-edge"
    "ungoogled-chromium"
    "yandex-browser"
    "sidekick"
    "helium"
    "net.imput.helium"
)

# Loop semua user dengan UID >= 1000
for home in /home/*; do
    [ ! -d "$home" ] && continue
    user=$(basename "$home")

    # Skip jika bukan user valid
    id "$user" >/dev/null 2>&1 || continue

    # Standard browsers
    for browser in "${BROWSERS[@]}"; do
        nmh_dir="${home}/.config/${browser}/NativeMessagingHosts"
        mkdir -p "$nmh_dir" 2>/dev/null || continue
        echo "$NMH_JSON" > "${nmh_dir}/com.fastdm.native.json" 2>/dev/null || true
        chown "${user}:${user}" "${nmh_dir}/com.fastdm.native.json" 2>/dev/null || true
        chown "${user}:${user}" "${nmh_dir}" 2>/dev/null || true

        # Fix parent chown juga
        parent=$(dirname "$nmh_dir")
        chown "${user}:${user}" "$parent" 2>/dev/null || true
    done

    # Ice/Helium profiles
    for ice_base in \
        "${home}/.local/share/ice/profiles" \
        "${home}/.local/share/helium/profiles"; do

        [ ! -d "$ice_base" ] && continue

        for profile in "$ice_base"/*/; do
            [ ! -d "$profile" ] && continue

            nmh_dir="${profile}NativeMessagingHosts"
            mkdir -p "$nmh_dir" 2>/dev/null || continue
            echo "$NMH_JSON" > "${nmh_dir}/com.fastdm.native.json" 2>/dev/null || true
            chown "${user}:${user}" "${nmh_dir}/com.fastdm.native.json" 2>/dev/null || true
            chown -R "${user}:${user}" "$nmh_dir" 2>/dev/null || true
        done
    done
done

# 3. System-wide (fallback untuk Chromium yang di-install system-wide)
for sys_dir in \
    "/etc/opt/chrome/native-messaging-hosts" \
    "/etc/chromium/native-messaging-hosts"; do
    mkdir -p "$sys_dir" 2>/dev/null || true
    echo "$NMH_JSON" > "${sys_dir}/com.fastdm.native.json" 2>/dev/null || true
done

# 4. Update caches
gtk-update-icon-cache -f /usr/share/icons/hicolor/ 2>/dev/null || true
update-desktop-database /usr/share/applications/ 2>/dev/null || true

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  ✓ Fast Download Manager v2.1.0 installed!"
echo "═══════════════════════════════════════════════════════"
echo ""
echo "  Native messaging host registered for browsers:"
echo "    • Chrome, Chromium, Thorium, Helium"
echo "    • Brave, Vivaldi, Opera, Edge"
echo "    • Ice/Helium WebApp profiles"
echo ""
echo "  Next steps:"
echo "    1. Load extension: /opt/fast-dm/extension"
echo "    2. Run: fast-dm"
echo "    3. Extension auto-connects ✓"
echo ""
echo "═══════════════════════════════════════════════════════"
echo ""

exit 0
POST
chmod 755 "${DEB_ROOT}/DEBIAN/postinst"

# prerm
cat > "${DEB_ROOT}/DEBIAN/prerm" << 'PRERM'
#!/bin/bash
rm -f /tmp/fast-dm*.sock 2>/dev/null || true
exit 0
PRERM
chmod 755 "${DEB_ROOT}/DEBIAN/prerm"

# Build deb
DEB_FILE="${DIST_DIR}/${APP_NAME}_${APP_VERSION}_${ARCH}.deb"
fakeroot dpkg-deb --build "${DEB_ROOT}" "${DEB_FILE}"

echo ""
echo "Package: ${DEB_FILE} ($(du -sh "$DEB_FILE" | awk '{print $1}'))"
echo ""
echo "Install: sudo dpkg -i ${DEB_FILE}"
echo "Run:     fast-dm"
