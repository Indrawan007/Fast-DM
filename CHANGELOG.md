# Changelog

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/),
versi mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [2.5.0] - 2026-09-02

### Added — D2: dialog "Simpan Sebagai…" (IDM-style)
- Tombol baru di toolbar: membuka `gtk4::FileDialog::save_file` dengan nama
  terduga dari URL + folder awal = Download dari Pengaturan; unduhan dimulai
  ke path pilihan user (engine tetap men-sanitasi nama & membuat folder).
- Untuk URL video (YouTube/HLS/halaman), dialog kualitas TETAP menyusul setelah
- Catatan API: gtk4-rs 0.9 menamai callback FileDialog `save()` (bukan
  `save_file` — itu alias dokumentasi C); mengikuti pola `select_folder` settings.
  file dipilih — satu alur, tanpa duplikasi logika.
- Refactor: normalisasi URL & keputusan "perlu dialog kualitas?" (B20)
  diekstrak ke `normalize_url_input` / `wants_quality_dialog` — dipakai bersama
  tombol Unduh, clipboard banner, dan Simpan Sebagai; +3 unit test.
- Batal pada dialog file = tidak ada aksi (konsumen UI tidak lagi kebingungan
  "URL kosong" seperti jalur Unduh manual).

## [2.4.0] - 2026-09-02

### Added — fitur D1 & D3 (roadmap CODE-REVIEW.md)
- **D3 Dukungan proxy global** — satu kolom di Pengaturan (`proxy_url`)
  diterapkan ke SEMUA engine: aria2c via `--all-proxy=`, yt-dlp (jalur YouTube
  dan resolver universal) via `--proxy`. Mendukung `http://`, `https://`,
  `socks4/4a/5/5h` termasuk kredensial di URL (`http://user:pass@host:port`).
  Nilai invalid ditolak saat Simpan (validasi skema+host) dengan pesan jelas.
- **D1 Deteksi URL dari clipboard ala IDM** — toggle "Deteksi URL unduhan dari
  clipboard" (default OFF, opt-in). Saat aktif, URL `http(s)` yang disalin di
  clipboard memunculkan banner di bawah toolbar dengan tombol **Unduh** (langsung
  masuk alur tambah-download normal — termasuk dialog kualitas YouTube) dan ✕
  untuk menutup. Implementasi polling CLI (`wl-paste` di Wayland, `xclip` di
  X11) — tanpa dependensi crate baru; dedup konten & cap 2 KB anti-spam;
  bila tool tidak terpasang, polling berhenti sendiri tanpa mengganggu.
- Config: `proxy_url` + `clipboard_monitor` dengan `#[serde(default)]` —
  `config.json` lama tetap terbaca utuh saat upgrade (regression test).

## [2.3.2] - 2026-09-02

### Changed — M4: dialog GUI jadi event-driven (tanpa nested main loop)
- `show_settings_dialog` & `show_quality_dialog` tidak lagi memblokir dengan
  `while dialog.is_visible() { main_context.iteration(true) }` — pola nested
  main loop berisiko reentrancy (event diproses ganda, dua dialog bisa saling
  spin). Keduanya kini menerima callback `FnOnce` yang dipanggil dari sinyal
  `response`: "Simpan"/"Unduh" → aksi, "Batal"/tutup → tidak ada aksi.
- Guard `connect_close_request` (anti loop menggantung) dihapus — tidak ada
  loop lagi.

### Fixed
- Dialog kualitas YouTube: menekan **Batal** dulu tetap memulai download
  (hasil `None` diperlakukan "tanpa kualitas"); kini batal sungguh membatalkan.

## [2.3.1] - 2026-09-02

### Changed — M1: proses anak downloader jadi async penuh (`tokio::process`)
- `aria2.rs` + `youtube.rs`: pola `std::process` di dalam `spawn_blocking` +
  `Handle::current().block_on` per baris output diganti reader baris async
  (`ChildLines`, cancellation-safe), **ticker 500 ms** untuk cek pause/cancel
  — tombol kini bekerja walau child tidak mengeluarkan output (stall jaringan),
  **wait paus terbatas 30 dtk** + eskalasi SIGKILL ke process group (dulu bisa
  menggantung permanen bila child mengabaikan SIGTERM), dan `kill_on_drop`
  sebagai jaring pengaman; stderr tidak lagi butuh thread khusus.
- Kill group kini dijaga `Option<pid>`: `killpg(0)` (bahaya: group Fast-DM
  sendiri) tidak mungkin lagi terjadi bila proses sudah selesai.

### Fixed
- `universal.rs`: dua pemanggilan `Handle::current().block_on` **dari dalam
  konteks async** (jalur yt-dlp-missing dan reset-fallback) — keduanya panic
  "Cannot block the current thread from within a runtime" saat tereksekusi;
  diganti await langsung.
- Koreksi kompilasi v2.3.0 (sudah masuk di `787a260`): tipe stream peer-cred
  ipc, trait `FileTypeExt`, perbandingan uid nix 0.29, `&dir` di setup NMH.

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
