# Fast-DM — Arch Linux + Hyprland

Fast-DM adalah download manager GTK4 untuk **Arch Linux x86_64 pada sesi Hyprland/Wayland**. Unduhan ditangani oleh `aria2` dan `yt-dlp`; ekstensi Chromium dapat mengirim tautan langsung dari browser.

GUI Fast-DM menggunakan backend GTK Wayland secara eksplisit. **Sesi X11/XWayland dan paket distro selain Arch bukan target dukungan proyek ini.** CI membangun dan menguji paket di Arch Linux; compositor Hyprland tidak dijalankan di CI.

## Fitur

- Unduhan multi-koneksi, jeda/lanjutkan, retry, dan pemulihan sesi melalui `aria2`.
- Tangga eskalasi untuk unduhan yang ditolak server (HTTP 403/login/anti-bot): retry dengan cookie/header terbaru dari browser, lalu yt-dlp dengan impersonasi sidik jari TLS Chrome — unduhan tetap ditangani Fast-DM (multi-koneksi + bisa dilanjut), dan penyerahan ke browser hanya jalan terakhir yang bisa dimatikan dari Pengaturan.
- YouTube dan banyak situs lain melalui `yt-dlp`, termasuk pilihan kualitas serta audio/video.
- Ekstensi Chromium untuk mencegat unduhan, membawa cookie/Referer, dan menampilkan pilihan kualitas di pemutar video.
- Pemantauan clipboard Wayland secara opsional melalui `wl-paste`.
- Pengaturan folder, batas kecepatan, koneksi, proxy, verifikasi TLS, dan pilihan menyerahkan ke browser bila server terus menolak Fast-DM.
- Integrasi browser Native Messaging dan notifikasi desktop opsional.
- Layout adaptif untuk Hyprland: toolbar, statistik, rincian unduhan, dan tombol kartu membungkus otomatis saat jendela ditile sempit; Pengaturan dapat digulir dan labelnya ikut membungkus.

## Dependensi Arch + Hyprland

Dependensi utama dipasang otomatis oleh paket Fast-DM. Untuk instalasi dari source:

```bash
sudo pacman -S --needed gtk4 aria2 yt-dlp ffmpeg xdg-utils wl-clipboard
```

Opsional untuk integrasi desktop Hyprland/GTK yang lebih lengkap:

```bash
sudo pacman -S --needed libnotify xdg-desktop-portal-hyprland xdg-desktop-portal-gtk
```

`wl-clipboard` menyediakan `wl-paste` untuk fitur pemantauan clipboard. Fitur ini mati secara default dan dapat diaktifkan melalui **Pengaturan**.

## Instalasi

1. Unduh paket `fast-dm-<versi>-1-x86_64.pkg.tar.zst` dari [rilis terbaru](https://github.com/Indrawan007/Fast-DM/releases/latest).
2. Pasang paket:

   ```bash
   sudo pacman -U ./fast-dm-*-x86_64.pkg.tar.zst
   ```

3. Jalankan **Fast Download Manager** dari launcher Hyprland, atau dari terminal:

   ```bash
   fast-dm
   ```

GUI memerlukan `WAYLAND_DISPLAY` dari sesi Wayland dan selalu memakai `GDK_BACKEND=wayland`; tidak ada fallback ke X11.

### Ekstensi browser

1. Unduh `fast-dm-extension-v<versi>.zip` dari halaman rilis dan ekstrak ke folder permanen, misalnya `~/.local/share/fast-dm-extension/`.
2. Buka `chrome://extensions/`, aktifkan **Developer mode**, lalu pilih **Load unpacked** dan arahkan ke folder hasil ekstrak.
3. Jalankan Fast-DM sekali agar Native Messaging terdaftar, lalu muat ulang ekstensi dan restart browser.

Ekstensi ditujukan untuk Chrome, Chromium, Brave, Edge, Opera, dan Vivaldi.

## Membangun dari source di Arch

Jalankan sebagai pengguna biasa, bukan `root`:

```bash
sudo pacman -S --needed base-devel rust gtk4 aria2 yt-dlp ffmpeg xdg-utils wl-clipboard nodejs npm

cargo fmt --all -- --check
cargo test --locked
cargo build --release --locked
npm ci
npm test
npm run lint
bash packaging/build-arch.sh
```

Paket Arch hasil build disimpan di `build/`. `makepkg` tidak boleh dijalankan sebagai `root`.

## Rilis

Artefak resmi hanya mencakup:

- `fast-dm-<versi>-1-x86_64.pkg.tar.zst` — aplikasi untuk Arch Linux.
- `fast-dm-extension-v<versi>.zip` — ekstensi browser.
- `SHA256SUMS` — checksum artefak.

Lihat [CHANGELOG](CHANGELOG.md) untuk riwayat perubahan dan [LICENSE](LICENSE) untuk lisensi MIT.
