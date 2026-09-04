[![CI](https://github.com/Indrawan007/Fast-DM/actions/workflows/ci.yml/badge.svg)](https://github.com/Indrawan007/Fast-DM/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Fast-DM adalah aplikasi Download Manager untuk Linux dengan dukungan browser extension untuk mempermudah pengiriman tautan unduh ke aplikasi.

## Fitur

- 🚀 **Download accelerator** via `aria2c` (multi-connection, segment, resume, limit global live via daemon RPC)
- 🎬 **YouTube & 1800+ situs** via `yt-dlp` (TikTok, IG, FB, X, Vimeo, HLS/DASH)
- 🔌 **Browser extension** (Chrome/Chromium/Brave/Edge/Opera/Vivaldi) dengan Native Messaging
- 🎯 **Overlay IDM-like** di YouTube player — klik ⚡ pilih kualitas
- 🍪 **Cookie per-domain** — download login-protected dari subdomain CDN
- 🌑 **Tema Catppuccin Mocha** untuk GTK4 GUI
- ⏸️ **Pause/resume/cancel** dengan SIGTERM (resume-friendly, bukan kill paksa)
- 📋 **Session persist** — unduhan yang belum selesai otomatis di-resume saat restart (bisa dimatikan di Pengaturan)
- 🔒 **IPC lokal aman** — socket di `XDG_RUNTIME_DIR` (0700) + verifikasi UID peer; cookies & file token tidak pernah ditulis ke `/tmp` publik
- 🌐 **Proxy global** (HTTP/SOCKS5, kredensial di URL) — satu kolom di Pengaturan, berlaku untuk aria2 & yt-dlp
- 📋 **Clipboard monitor** (opt-in) — URL yang disalin terdeteksi otomatis dengan banner "Unduh", ala IDM

## Download

👉 https://github.com/Indrawan007/Fast-DM/releases/latest

### Release Files

- `fast-dm_<versi>_amd64.deb` — aplikasi Linux
- `fast-dm-extension-v<versi>.zip` — browser extension

## Instalasi

### Linux App

```bash
sudo apt install ./fast-dm_*_amd64.deb
```

### Browser Extension

1. Download `fast-dm-extension-v<versi>.zip` dari release
2. Extract ke folder permanen (mis. `~/.local/share/fast-dm-extension/`)
3. Buka `chrome://extensions/` → aktifkan **Developer mode**
4. Klik **Load unpacked** → pilih folder hasil extract

Catatan: ID extension akan otomatis ter-register di native messaging manifest saat pertama kali load.

## Development

### Build dari source

```bash
# Dependensi sistem (Ubuntu/Debian)
sudo apt install build-essential libgtk-4-dev aria2 yt-dlp ffmpeg xdg-utils

# Build release
cargo build --release

# Jalankan tests
cargo test

# Buat .deb
bash packaging/build-deb.sh
```

### Struktur Kode

- `src/lib.rs` — library crate (semua module publik)
- `src/main.rs` — binary entry point (CLI dispatch: GUI / NMH)
- `src/downloader/` — `aria2` (jalur per-proses + pipeline resolve), `aria2_rpc` (daemon RPC: magnet & http/ftp), `youtube`, `universal` (resolver), `mod` (engine)
- `src/ipc/` — Unix socket server untuk browser → GUI
- `src/native_host/` — Chrome Native Messaging wrapper
- `src/gui/` — GTK4 window & dialog
- `extension/` — Manifest V3 extension (background, content, sniffer, popup)
- `tests/` — integration test (filesystem terisolasi via `std::env::temp_dir()` + override XDG)

Lihat [CHANGELOG.md](CHANGELOG.md) untuk history rilis.

## Lisensi

MIT — lihat [LICENSE](LICENSE).
