# Changelog

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/),
versi mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [2.3.0] - 2026-09-02

### Fixed — keamanan (lihat CODE-REVIEW.md)
- **K1** IPC socket pindah dari `/tmp/fast-dm-<uid>.sock` (publik & bisa
  di-preempt user lain) ke `XDG_RUNTIME_DIR/fast-dm/fast-dm.sock` (0700) dengan
  fallback `~/.config/fast-dm/run/`. Koneksi diterima hanya bila `SO_PEERCRED`
  UID sama dengan UID aplikasi. Socket warisan versi lama di /tmp dibersihkan
  saat start (hanya milik user sendiri).
- **K2** `build.sh` (skrip build lama, tidak dipakai CI) dihapus — postinst-nya
  memasang `allowed_origins: chrome-extension://*/*` di native messaging
  manifest, yang mengizinkan ekstensi browser apa pun memanggil aplikasi.
  Jalur resmi tetap `packaging/build-deb.sh` + registrasi NMH oleh aplikasi.
- **K3** File input aria2c (berisi URL yang mungkin bertoken) kini ditulis ke
  direktori privat `0700` dengan file `0600`; sisa file sesi crash dibersihkan
  saat aplikasi menyala.
- **K4** Pause/cancel/exit kini mengirim sinyal ke **process group**
  (`killpg`) — anak `ffmpeg` yang di-spawn yt-dlp ikut berhenti, tidak lagi
  menjadi proses yatim yang terus menulis file.
- Cookie dari extension disimpan dengan TTL wajar + di-GC otomatis > 7 hari
  (sebelumnya ditulis dengan kedaluwarsa 1 tahun tanpa pembersihan).

### Changed
- **K5** Unduhan tertunda hasil restore sesi **benar-benar** dilanjutkan
  otomatis saat aplikasi dibuka (README sebelumnya mengklaim begitu padahal
  kode hanya menandai Paused). Dapat dimatikan lewat Pengaturan →
  "Lanjutkan otomatis…".
- Batas kecepatan total kini dibagi menurut jumlah unduhan **aktif+antri saat
  start** (sebelumnya selalu dibagi `max_concurrent` — unduhan tunggal hanya
  memakai sepertiga limit).
- `session.json` memakai format berversi (`{"version":1,…}`); file korup
  dibackup sebagai `session.json.corrupt-<ts>` alih-alih hilang diam-diam.

### Added
- Ekstensi yang dikenali untuk jalur-download-cepat (`.exe .msi .dmg .bz2
  .docx …`) diselaraskan dengan daftar intersep browser — URL non-media tak
  lagi mencoba yt-dlp dulu (hemat 1–3 detik).
- Deteksi URL YouTube lebih toleran (`/live/`, `/embed/`, `/v/`, query param
  `v=` di posisi mana pun) → dialog kualitas muncul untuk format tersebut.
- Skema URL non-http(s)/ftp ditolak cepat dengan pesan jelas (sebelumnya
  spawned lalu gagal lambat di CLI).
- `LICENSE` (MIT — selama ini direferensikan README tapi tidak ada) dan
  changelog ini.
- Info non-error ("Merging video + audio…") pindah ke `status_detail` — tidak
  lagi menyamar sebagai pesan error merah di kartu unduhan.
- Checkbox "Lanjutkan otomatis unduhan tertunda" di dialog Pengaturan.

## [2.2.5] - sebelumnya
- Perbaikan bug various (lihat riwayat commit), Catppuccin GUI, session
  persist, cookie per-domain, resolver universal yt-dlp + fallback aria2.
