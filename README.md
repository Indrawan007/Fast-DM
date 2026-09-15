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
- 🔒 **IPC lokal aman** — socket di `XDG_RUNTIME_DIR` (0700) + verifikasi UID peer + allow-list header dari extension; cookies & file token tidak pernah ditulis ke `/tmp` publik; extension ID baru yang diizinkan memanggil native host diumumkan lewat notifikasi desktop
- 🌐 **Proxy global** (HTTP/SOCKS5, kredensial di URL) — satu kolom di Pengaturan, berlaku untuk aria2 & yt-dlp
- 📋 **Clipboard monitor** (opt-in) — URL yang disalin terdeteksi otomatis dengan banner "Unduh", ala IDM

## Perubahan v3.0.0

**Release breaking (semver major): fitur download torrent & magnet link dihapus.**

- Skema `magnet:` kini ditolak dengan pesan jelas — "Skema URL tidak didukung
  — http, https, atau ftp." — di gate engine maupun IPC extension (sama
  seperti `blob:`/`data:` dan skema non-download lain).
- Deteksi `magnet:`, flag `--bt-*`/`--seed-time`, dan tampilan seeders/peers
  dihapus dari jalur daemon RPC. **Daemon RPC tetap dipakai** untuk unduhan
  http/https/ftp (limit kecepatan global live & pause/resume native).
- File metafile (`.torrent`, `.nzb`, `.metalink`, `.meta4`) tetap di daftar
  297 ekstensi file langsung dan tetap di-intercept extension, tetapi kini
  **diunduh sebagai file biasa** — kedua jalur aria2 memakai
  `--follow-torrent=false` / opsi `follow-torrent: "false"` sehingga aria2
  tidak lagi mengikuti metadata-nya (default aria2 sebelumnya malah mengunduh
  konten yang dideskripsikan torrent-nya).
- Clipboard monitor tidak lagi memicu untuk `magnet:`. Input magnet yang
  ditempel tetap lolos normalisasi apa adanya agar penolakan engine memakai
  pesan skema yang jelas (bukan URL `https://magnet:…` sampah).

## Stabilitas v2.11.2

- **Unduhan hidup lagi** — nilai `min-split-size` `512K` (jalur per-proses DAN
  opsi per-URI daemon) berada di luar rentang sah aria2 (`1M`–`1024M`), sehingga
  aria2c keluar dengan exit code 28 sebelum mengunduh satu byte pun dan
  `aria2.addUri` fault di jalur daemon; semua unduhan http/ftp serta magnet mati
  sejak v2.10.5. Kini keduanya memakai konstanta `MIN_SPLIT_SIZE = "1M"` (nilai
  terkecil yang sah = split paling agresif yang diizinkan), dijaga test
  `min_split_size_within_aria2_documented_range` agar tidak "dioptimalkan" ke
  bawah rentang lagi.

## Stabilitas v2.11.1

Rilis perbaikan — tidak ada fitur baru, tidak ada perubahan antarmuka.

- **Homepage tidak lagi dianggap file** — URL tanpa path (`https://x.com`,
  `https://cdn.example.com`, `https://sub.domain.com`, `https://x.com:8080`,
  `https://user:pass@host.com`) dulu lolos sebagai "file langsung" karena
  segmen terakhirnya adalah _host_, dan `.com` (executable DOS) memang ada di
  daftar ekstensi. Halaman seperti itu kini benar-benar lewat resolver
  universal (yt-dlp) lebih dulu. Ekstensi dibaca dari **path** saja: helper
  `url_path_part` membuang `skema://user:pass@host:port`. File `.com` sungguhan
  (`https://x.com/game.com`) tetap terdeteksi.
- **Tombol "Pindai" menangkap semua format audio** — daftar media
  `content.js` disamakan dengan `sniffer.js`/`background.js` (59 → 86; 27
  format audio yang hilang ikut tertangkap), dan `href` berfragment
  (`.../v.mp4#t=10`) kini juga terdeteksi.
- **CI hijau** — 4 test extension yang gagal sejak v2.11.0 diperbaiki (mock
  `chrome.downloads.onDeterminingFilename` belum ada): suite Node kini 8/8.
- **Versi rilis dijaga test** — `tests/version_sync.rs` memastikan
  `Cargo.toml`, `Cargo.lock`, dan `extension/manifest.json` selalu sama, jadi
  bump versi yang lupa satu berkas gagal di `cargo test`, bukan diam-diam
  terkirim. `packaging/build-deb.sh` kini memakai `--locked` seperti CI.
- Kebersihan: 11 test di `youtube.rs` masuk ke `#[cfg(test)]` (tidak lagi ikut
  terkompilasi ke build rilis), cabang fallback mati dihapus, komentar basi dan
  typo diperbaiki.

## Stabilitas v2.11.0

- **297 jenis file** dikenali langsung → aria2 (video, audio, gambar, arsip, dokumen, installer, VM, torrent, font, 3D, DB) — dulu hanya ~50
- **Extension intercept** diperluas: 86 video + 213 file (total 299 inc. m3u8/mpd) → semua jenis file dari situs apapun ter-intercept
- **Sniffer media** 15 → 86 format (m3u8/mpd/mp4/mkv/webm/flv/avi/mov/mp3/flac/ogg/opus/dll)
- **RAM hemat**: disk cache 64M→32M + `--enable-mmap=true`
- **CPU hemat**: `--optimize-concurrent-downloads=true`, piece 1M
- **Speed naik**: bt peers 55→100 + LPD, 5 concurrent (dari 3), yt-dlp chunk 10M + buffer 16K, fragment paralel mengikuti koneksi per server

## Stabilitas v2.10.5

- Fragmen HLS/DASH diunduh paralel (`--concurrent-fragments`, mengikuti
  "Koneksi per server") — kecepatan situs streaming naik signifikan.
- Merge video+audio memakai MKV (remux tanpa re-encode); MP4 hanya untuk
  pilihan MP4/audio eksplisit. Tidak ada lagi re-encode lambat untuk stream
  webm/VP9/AV1.
- Unduhan magnet langsung selesai (seeding dinonaktifkan lewat `--seed-time=0`).
- Format video-only dari dialog kualitas otomatis dipasangkan dengan audio.
- Dialog kualitas muncul seketika; daftar format nyata menyusul secara asinkron.

## Stabilitas v2.10.4

- Lanjut Semua mengajukan resume unduhan paused/error menurut waktu pembuatan
  (terlama dahulu), dengan ID sebagai pembanding ketika waktunya sama.
  Urutan pengajuan tidak lagi bergantung pada iterasi HashMap; batas slot dan
  pemeriksaan status tetap ditangani engine seperti sebelumnya.

## Stabilitas v2.10.3

- Tombol Jeda/Lanjut Semua memperhitungkan antrean dan resume/retry tertunda,
  bukan hanya unduhan yang sedang mentransfer data.
- Aksi tombol dibaca ulang dari engine saat diklik; klik berulang diblokir selama
  operasi massal masih berlangsung.
- Jeda individual maupun Jeda Semua membatalkan retry tertunda pada status Error
  tanpa mengubah unduhan gagal biasa yang belum diminta retry.

## Stabilitas v2.10.2

- Start/resume berulang tidak membuat worker unduhan ganda. Resume yang diminta
  saat backend masih berhenti akan menunggu cleanup selesai; pause ulang/cancel
  membatalkan permintaan tersebut.
- Worker yang sedang berhenti tetap memakai slot antrean. Item antrean juga dapat
  dijeda satu per satu.
- Unduhan yang dipromosikan dari antrean menggunakan konfigurasi terbaru.
  Proses yang sudah aktif tidak direstart untuk mengganti seluruh argumennya.
- Shutdown tidak mempromosikan pekerjaan baru. Flag lifecycle worker tidak
  dipersistensikan sehingga tidak menghalangi pemulihan sesi berikutnya.

## Stabilitas v2.10.1

- Proxy HTTP/SOCKS juga digunakan saat memeriksa nama dan ukuran file; client
  resolver mengikuti perubahan pengaturan proxy/TLS.
- Pengaturan verifikasi TLS berlaku konsisten untuk metadata dan unduhan yt-dlp.
  Verifikasi tetap aktif secara default; nonaktifkan hanya untuk server tepercaya.
- Penambahan unduhan bersamaan melakukan pemeriksaan duplikat dan penyisipan
  secara atomik.
- Monitor clipboard berjalan asinkron, dengan timeout 1 detik per perintah dan
  batas output 2 KiB, agar tool clipboard yang macet tidak membekukan GUI.
- Tombol unduh hasil pemindaian popup menunggu konfirmasi aplikasi, menampilkan
  kegagalan, dan menyediakan kesempatan mencoba lagi.

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

# Regression test extension (Node.js 22+, tanpa npm install)
node --test tests/extension.test.cjs

# Buat .deb
bash packaging/build-deb.sh
```

### Struktur Kode

- `src/lib.rs` — library crate (semua module publik)
- `src/main.rs` — binary entry point (CLI dispatch: GUI / NMH)
- `src/downloader/` — `aria2` (jalur per-proses + pipeline resolve), `aria2_rpc` (daemon RPC http/ftp: limit global live, pause/resume native; v3.0.0: jalur magnet dihapus), `youtube`, `universal` (resolver), `mod` (engine)
- `src/ipc/` — Unix socket server untuk browser → GUI
- `src/native_host/` — Chrome Native Messaging wrapper
- `src/gui/` — GTK4 window & dialog
- `extension/` — Manifest V3 extension (background, content, sniffer, popup)
- `tests/` — integration test: `find_cookies.rs` (filesystem terisolasi via `std::env::temp_dir()` + override `XDG_CONFIG_HOME`, serial lewat `ENV_LOCK`), `version_sync.rs` (versi `Cargo.toml`/`Cargo.lock`/`manifest.json` harus sama; tanpa I/O runtime — berkas disematkan `include_str!`), `extension.test.cjs` (Node 22+, mock `chrome.*` lewat `vm`)

Lihat [CHANGELOG.md](CHANGELOG.md) untuk history rilis.

## Lisensi

MIT — lihat [LICENSE](LICENSE).
