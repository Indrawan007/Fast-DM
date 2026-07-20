#!/bin/bash
#
# Fast Download Manager - Installer untuk Linux Mint / Ubuntu
#
# Apa yang dilakukan:
# 1. Install aria2 (jika belum ada)
# 2. Setup Python environment
# 3. Register Chrome Native Messaging Host
# 4. Create desktop entry
# 5. Generate extension icons
#

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="fast-dm"
NATIVE_HOST_NAME="com.fastdm.native"

echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════════╗"
echo "║     ⚡ Fast Download Manager Installer        ║"
echo "╚═══════════════════════════════════════════════╝"
echo -e "${NC}"

# ===== 1. Check & Install aria2 =====
echo -e "${YELLOW}[1/6] Checking aria2...${NC}"
if command -v aria2c &> /dev/null; then
    ARIA2_VER=$(aria2c --version | head -1)
    echo -e "${GREEN}  ✓ $ARIA2_VER${NC}"
else
    echo -e "${YELLOW}  Installing aria2...${NC}"
    sudo apt-get update -qq
    sudo apt-get install -y -qq aria2
    echo -e "${GREEN}  ✓ aria2 installed${NC}"
fi

# ===== 2. Check Python3 =====
echo -e "${YELLOW}[2/6] Checking Python3...${NC}"
if command -v python3 &> /dev/null; then
    PY_VER=$(python3 --version)
    echo -e "${GREEN}  ✓ $PY_VER${NC}"
else
    echo -e "${RED}  ✕ Python3 not found! Please install python3.${NC}"
    exit 1
fi

# Check GTK bindings
echo -e "${YELLOW}  Checking GTK3 Python bindings...${NC}"
if python3 -c "import gi; gi.require_version('Gtk', '3.0'); from gi.repository import Gtk" 2>/dev/null; then
    echo -e "${GREEN}  ✓ GTK3 Python bindings found${NC}"
else
    echo -e "${YELLOW}  Installing python3-gi...${NC}"
    sudo apt-get install -y -qq python3-gi python3-gi-cairo gir1.2-gtk-3.0
    echo -e "${GREEN}  ✓ GTK3 Python bindings installed${NC}"
fi

# ===== 3. Generate Icons =====
echo -e "${YELLOW}[3/6] Generating extension icons...${NC}"
ICON_DIR="${SCRIPT_DIR}/extension/icons"
mkdir -p "$ICON_DIR"

# Generate simple SVG icon and convert to PNG
# Using Python to create icons (no ImageMagick dependency)
python3 << 'ICONSCRIPT'
import os

svg_template = '''<?xml version="1.0" encoding="UTF-8"?>
<svg width="{size}" height="{size}" viewBox="0 0 {size} {size}" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#89b4fa"/>
      <stop offset="100%" style="stop-color:#74c7ec"/>
    </linearGradient>
  </defs>
  <rect width="{size}" height="{size}" rx="{radius}" fill="url(#bg)"/>
  <text x="50%" y="55%" text-anchor="middle" dominant-baseline="middle"
        font-family="sans-serif" font-weight="bold" font-size="{fontsize}"
        fill="#1e1e2e">⚡</text>
</svg>'''

script_dir = os.path.dirname(os.path.abspath(__file__)) if '__file__' in dir() else '.'
icon_dir = os.path.join(os.environ.get('SCRIPT_DIR', '.'), 'extension', 'icons')

for size in [16, 48, 128]:
    radius = max(2, size // 8)
    fontsize = size * 0.6
    svg = svg_template.format(size=size, radius=radius, fontsize=int(fontsize))
    svg_path = os.path.join(icon_dir, f'icon{size}.svg')
    with open(svg_path, 'w') as f:
        f.write(svg)
ICONSCRIPT

# Convert SVG to PNG if rsvg-convert is available
if command -v rsvg-convert &> /dev/null; then
    for size in 16 48 128; do
        rsvg-convert -w $size -h $size \
            "${ICON_DIR}/icon${size}.svg" \
            -o "${ICON_DIR}/icon${size}.png" 2>/dev/null || true
    done
    echo -e "${GREEN}  ✓ PNG icons generated${NC}"
else
    # Fallback: create minimal valid PNG files
    echo -e "${YELLOW}  rsvg-convert not found, installing librsvg2-bin...${NC}"
    sudo apt-get install -y -qq librsvg2-bin 2>/dev/null || true
    if command -v rsvg-convert &> /dev/null; then
        for size in 16 48 128; do
            rsvg-convert -w $size -h $size \
                "${ICON_DIR}/icon${size}.svg" \
                -o "${ICON_DIR}/icon${size}.png" 2>/dev/null || true
        done
        echo -e "${GREEN}  ✓ PNG icons generated${NC}"
    else
        # Create 1x1 pixel PNG as absolute fallback
        python3 << 'PNGSCRIPT'
import struct, zlib, os

def create_png(width, height, color=(137, 180, 250)):
    """Create a solid color PNG file."""
    def chunk(chunk_type, data):
        c = chunk_type + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xFFFFFFFF)

    header = b'\x89PNG\r\n\x1a\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))

    raw_data = b''
    for y in range(height):
        raw_data += b'\x00'  # filter none
        for x in range(width):
            raw_data += bytes(color)

    idat = chunk(b'IDAT', zlib.compress(raw_data))
    iend = chunk(b'IEND', b'')
    return header + ihdr + idat + iend

icon_dir = os.path.join(os.environ.get('SCRIPT_DIR', '.'), 'extension', 'icons')
for size in [16, 48, 128]:
    png_path = os.path.join(icon_dir, f'icon{size}.png')
    if not os.path.exists(png_path):
        with open(png_path, 'wb') as f:
            f.write(create_png(size, size))
PNGSCRIPT
        echo -e "${GREEN}  ✓ Fallback icons generated${NC}"
    fi
fi

# ===== 4. Create Native Messaging Host manifest =====
echo -e "${YELLOW}[4/6] Registering Chrome Native Messaging Host...${NC}"

# Chrome native messaging host directory
CHROME_NMH_DIR="$HOME/.config/google-chrome/NativeMessagingHosts"
CHROMIUM_NMH_DIR="$HOME/.config/chromium/NativeMessagingHosts"

# Create the native host script
NATIVE_HOST_SCRIPT="${SCRIPT_DIR}/native_host_entry.sh"
cat > "$NATIVE_HOST_SCRIPT" << NHEOF
#!/bin/bash
# Fast DM Native Messaging Host Entry Point
exec python3 "${SCRIPT_DIR}/main.py" --native
NHEOF
chmod +x "$NATIVE_HOST_SCRIPT"

# Create NMH manifest JSON
# NOTE: "allowed_origins" harus diupdate dengan Extension ID setelah install
NMH_MANIFEST='{
  "name": "'${NATIVE_HOST_NAME}'",
  "description": "Fast Download Manager Native Host",
  "path": "'${NATIVE_HOST_SCRIPT}'",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://EXTENSION_ID_PLACEHOLDER/"
  ]
}'

# Install for Chrome
mkdir -p "$CHROME_NMH_DIR"
echo "$NMH_MANIFEST" > "${CHROME_NMH_DIR}/${NATIVE_HOST_NAME}.json"
echo -e "${GREEN}  ✓ Chrome native host registered${NC}"

# Install for Chromium
mkdir -p "$CHROMIUM_NMH_DIR"
echo "$NMH_MANIFEST" > "${CHROMIUM_NMH_DIR}/${NATIVE_HOST_NAME}.json"
echo -e "${GREEN}  ✓ Chromium native host registered${NC}"

# ===== 5. Create Desktop Entry =====
echo -e "${YELLOW}[5/6] Creating desktop entry...${NC}"

DESKTOP_FILE="$HOME/.local/share/applications/${APP_NAME}.desktop"
cat > "$DESKTOP_FILE" << DEOF
[Desktop Entry]
Name=Fast Download Manager
Comment=High-speed download manager with Chrome integration
Exec=python3 ${SCRIPT_DIR}/main.py
Icon=${ICON_DIR}/icon128.png
Terminal=false
Type=Application
Categories=Network;FileTransfer;
Keywords=download;manager;video;
StartupNotify=true
DEOF

chmod +x "$DESKTOP_FILE"
echo -e "${GREEN}  ✓ Desktop entry created${NC}"

# ===== 6. Make main.py executable =====
echo -e "${YELLOW}[6/6] Finalizing...${NC}"
chmod +x "${SCRIPT_DIR}/main.py"

echo ""
echo -e "${GREEN}╔═══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║     ✓ Installation Complete!                  ║${NC}"
echo -e "${GREEN}╚═══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Next steps:${NC}"
echo ""
echo -e "  1. ${YELLOW}Load the Chrome Extension:${NC}"
echo -e "     • Open Chrome → chrome://extensions"
echo -e "     • Enable 'Developer mode'"
echo -e "     • Click 'Load unpacked'"
echo -e "     • Select: ${SCRIPT_DIR}/extension"
echo ""
echo -e "  2. ${YELLOW}Update Extension ID:${NC}"
echo -e "     • After loading, copy the Extension ID from chrome://extensions"
echo -e "     • Run: ${GREEN}bash ${SCRIPT_DIR}/set_extension_id.sh YOUR_EXTENSION_ID${NC}"
echo ""
echo -e "  3. ${YELLOW}Start Fast DM:${NC}"
echo -e "     • From menu: Search 'Fast Download Manager'"
echo -e "     • Or terminal: ${GREEN}python3 ${SCRIPT_DIR}/main.py${NC}"
echo ""
echo -e "  4. ${YELLOW}Test:${NC}"
echo -e "     • Click the ⚡ icon in Chrome toolbar"
echo -e "     • Right-click any link → 'Download with Fast DM'"
echo ""
