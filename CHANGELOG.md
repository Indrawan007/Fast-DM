# Changelog

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/),
versi mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [2.9.2] - 2026-09-04

### Fixed

- **Resume RPC setelah daemon/GID hilang tidak lagi macet** — hasil `addUri`
  pengganti sekarang selalu di-`unpause`; kegagalan unpause membersihkan task
  sebelum fallback, sehingga UI tidak terjebak pada status Mengunduh dengan
  task daemon yang sebenarnya masih paused.
- **Cancel/Hapus item RPC yang sudah dijeda benar-benar membersihkan daemon** —
  engine kini memanggil `forceRemove` langsung karena supervisor polling item
  paused sudah berhenti dan tidak dapat melakukan cleanup lagi.
- **“Hentikan & Tutup” mencakup daemon RPC** — shutdown mem-pause task,
  mematikan/reap daemon aria2, menghentikan process group subprocess, dan
  menulis snapshot session final secara terserialisasi. Status aktif tetap
  restorable untuk auto-resume, sedangkan PID/GID daemon mati tidak disimpan.
- **Batas request IPC 1 MB kini diterapkan saat membaca**, bukan setelah
  `read_line` mengalokasikan seluruh payload; peer tidak dapat menumbuhkan
  buffer tanpa batas sebelum validasi.
- **GitHub Actions kembali valid** — nama step yang mengandung titik dua
  sekarang dikutip dan step Clippy duplikat sebelum instalasi toolchain
  dihapus. Komentar jumlah test yang sudah basi juga dinetralkan.
- Sinkronisasi dokumentasi: README tidak lagi mengklaim memakai crate
  `tempfile`; path runtime, status CI, dan klaim jumlah test di
  `CODE-REVIEW.md` kini sesuai implementasi aktif (bagian historis tetap
  ditandai jelas).

### Changed

- `session.json` kini ditulis dengan permission `0600`; kegagalan flush
  periodik dicoba ulang pada tick berikutnya.
- Versi package, lockfile, dan browser extension disinkronkan ke `2.9.2`.

## [2.9.1] - 2026-09-04

### Fixed — perbaikan pasca-migrasi daemon RPC (B2)

- **Magnet kembali bisa ditambahkan** — gate skema di `add_download` masih
  hanya mengizinkan http/https/ftp (sisa komentar "magnet = roadmap"),
  sehingga `magnet:?xt=…` dari GUI/ekstensi ditolak "Skema URL tidak
  didukung" sebelum sempat mencapai daemon RPC. Kini `magnet:` diterima
  (fungsi murni `is_supported_scheme`, +2 unit test).
- **Pause/resume RPC benar-benar native** — task daemon yang dijeda kini
  dipertahankan: GID disimpan di field baru `DownloadInfo.rpc_gid`
  (serde default, kompatibel dengan session.json lama), resume memanggil
  `unpause` pada GID yang SAMA. Sebelumnya resume memanggil `addUri`
  baru sementara task paused lama tertinggal macet di daemon (duplikat
  task + dua penulis potensial untuk file yang sama). GID yang hilang
  (daemon di-restart) otomatis jatuh ke `addUri` ulang; GID dibersihkan
  saat selesai/error/cancel.
- **Batas kecepatan total kini berlaku juga untuk jalur yt-dlp** (YouTube
  & resolver universal) lewat `--limit-rate` — sebelumnya flag limit di
  Pengaturan hanya diteruskan ke aria2.

### Changed — kebersihan & konsistensi

- Dialog kualitas (GTK + overlay ekstensi) memakai Bahasa Indonesia
  konsisten dengan seluruh UI ("Kualitas Terbaik", "Rendah", dst.) —
  sebelumnya berbahasa Inggris.
- Ekstensi: dua listener `chrome.runtime.onStartup` duplikat digabung
  jadi satu; fungsi mati `sendDownload` di content.js dihapus (permintaan
  unduhan dikirim inline dari handler).
- `native_host/setup.rs`: komentar terduplikasi/rusak dirapikan.

## [2.9.0] - 2026-09-04

### Added — B2.2: migrasi HTTP/HTTPS/FTP ke daemon RPC (selesainya jalur B2)

- Unduhan **file langsung** (http/https/ftp) kini berjalan lewat **daemon
  `aria2c --enable-rpc`** yang sama dengan magnet (B2.1) — bukan lagi proses
  `aria2c` per-unduhan:
  - **Limit total global benar-benar live untuk semua unduhan** — daemon
    membagi ulang ke semua yang aktif seketika (`changeGlobalOption`),
    menutup celah M3 sisa: proses lama yang tidak pernah di-recalculate;
  - **Pause/resume native** (`forcePause`/`unpause`) — parsian & state utuh
    di daemon; resume lintas sesi via deteksi GID + control file
    (`--auto-save-interval=20`);
  - Koneksi/DNS di-reuse antar-unduhan satu daemon (tanpa spawn proses baru);
  - `status_detail` "seeders/peers" kini hanya untuk torrent (http/ftp
    tidak menampilkannya lagi).
- **Nol regresi perilaku**: pipeline `aria2.rs` tetap dijalankan SEBELUM
  `addUri` — resolve filename (Content-Disposition/redirect/ekstensi),
  penolakan HTML & non-2xx, dan pre-check ruang disk. Cookie per-domain
  (walk-up) & header (mis. Referer) dikirim sebagai **opsi per-URI**
  `cookie`/`header` — daemon global tidak menyentuh domain lain. Opsi
  `timeout`/`max-tries`/`retry-wait`/`min-split-size`/`piece-length`/
  `allow-overwrite` mengikuti Pengaturan (builder `adduri_options` murni).
- **Fallback zero-regresi**: bila daemon tak bisa dipakai (mis. `rpc_port`
  bentrok) atau `addUri` ditolak SEBELUM unduhan berjalan, http/ftp otomatis
  jatuh ke jalur per-proses lama — unduhan tetap jalan. Magnet tetap
  RPC-only (error jelas bila daemon tak tersedia). Fallback resolver
  universal (yt-dlp gagal → aria2) tidak berubah (per-proses).
- +4 unit test (`adduri_options`: flag dasar, out/cookie/header, strip CRLF
  & skip kosong, mengikuti settings).

## [2.8.1] - 2026-09-03

### Changed — kebersihan kode + gerbang lint CI (roadmap "CI clippy")

- `cargo clippy --fix` menyapu lint mekanis (map_or ×10, collapsible-if ×4,
  useless_conversion, redundant_closure, dsb.).
- Manual: alias tipe `SharedInfo`/`DownloadMap` (type_complexity, 7
  signature), `#[derive(Default)]` FastDmApp, helper clipboard dipindah ke
  sebelum `mod tests` (items_after_test), clone dihapus dari `DownloadStatus`
  (Copy), test daemon_args → struct-update literal.
- Dok: pemisah baris kosong antara list dan paragraf menyusul (doc-list
  indentation).
- CI (`ci.yml`, diterapkan manual — sandbox tanpa izin workflows): komponen
  clippy + step `cargo clippy --all-targets -- -D warnings` advisory
  (continue-on-error). Flip menjadi gate blocking = hapus 1 baris tsb.
- Nol perubahan perilaku; jumlah test tetap 134.

## [2.8.0] - 2026-09-03

### Added — D8.1: minimize-to-close & autostart (tanpa dependensi baru)

- **`minimize_to_close`** (default OFF, opt-in di Pengaturan): menutup jendela
  saat masih ada unduhan aktif/antri → jendela disembunyikan, engine tetap
  jalan; dialog "Hentikan & Tutup" lama tidak muncul. Membuka lagi cukup
  menjalankan ulang `fast-dm` — mekanisme single-instance (app.rs) meneruskan
  activate ke proses pertama dan memanggil `present()`. Config dibaca saat
  tombol tutup ditekan → perubahan pengaturan langsung berlaku tanpa restart.
  Tanpa unduhan aktif, tutup = keluar (perilaku lama).
- **`autostart`**: checkbox "Jalankan Fast DM otomatis saat login" — menulis/
  menghapus `~/.config/autostart/fast-dm.desktop` (Exec = current_exe,
  dikutip bila mengandung spasi). Side-effect file hanya saat nilainya
  BERUBAH dan hanya setelah engine menerima config ( pola D1).
- Helper murni `should_minimize_on_close` + `desktop_entry_for` +
  `apply_autostart_in` (uji terisolasi di temp dir) — +4 unit test.
- Tray icon sungguhan (StatusNotifierItem) sengaja DITANGGUH: butuh dependensi
  C/D-Bus yang tidak terverifikasi di loop build ini; nilai utama D8 (download
  tidak mati saat jendela tertutup) sudah tercapai.

## [2.7.0] - 2026-09-03

### Added — B2.1: daemon RPC aria2 + unduh magnet/torrent

- Modul baru `downloader/aria2_rpc.rs`: klien JSON-RPC 2.0 (reqwest, tanpa
  dependensi baru) + supervisor daemon `aria2c --enable-rpc` — spawn sekali
  per sesi, `kill_on_drop`, reuse daemon yatim milik sendiri via probe
  `getVersion` ber-token.
- **Magnet akhirnya bisa**: `magnet:?…` yang sebelumnya ditolak semua backend
  kini masuk antrean normal (slot & antrian engine tetap berlaku) via
  `addUri` + poll `tellStatus` 600 ms; nama file diisi otomatis dari metadata
  begitu dikenal aria2 (hanya bila user tidak menentukan nama).
- **Limit total live**: saat unduhan RPC start, `changeGlobalOption`
  `max-overall-download-limit` disetel dari config — daemon membagi ulang
  sendiri ke semua unduhan aktif (perbaikan langsung keluhan M3 "tidak
  di-recalculate" untuk jalur RPC).
- Pause/resume = `forcePause`/`unpause` (parsian & state utuh di daemon,
  tanpa SIGKILL); cancel = `forceRemove` (parsial dibiarkan, konsisten jalur
  proses). GID lama dideteksi ulang oleh aria2 → resume lintas sesi jalan.
- Keamanan: RPC bind loopback + secret acak per install
  (`~/.config/fast-dm/rpc.secret`, mode 600) — daemon asing tak bisa
  mengontrol, daemon kita dari sesi lalu tetap ter-autentikasi.
- Config baru: `rpc_port` (default 6800, `#[serde(default)]` — config lama
  aman). URL `magnet:` lolos normalisasi input & tanpa dialog kualitas.
- Gate batch: http/https langsung TETAP lewat `aria2.rs` per-proses (nol
  regresi); migrasi penuh ke daemon = B2.2.
- +10 unit test murni (request/response/daemon-args/patch/secret/magnet).

## [2.6.1] - 2026-09-03

### Fixed

- Guard passthrough D6 (`looks_like_format_id`) keliru menerima kata bebas
  tanpa digit: quality basi ("unknown", "high") terkirim sebagai
  `--format unknown/best` alih-alih jatuh ke default. Kini wajib minimal satu
  digit ASCII (id format yt-dlp selalu numerik) — test `quality_args_default`
  dan `quality_args_non_numeric_p_ignored` (yang benar) kembali hijau.
- Warning `unused_mut` di `fetch_formats` (Child::wait_with_output mengonsumsi
  self, binding `mut` tidak diperlukan).

## [2.6.0] - 2026-09-02

### Added — D6: dialog kualitas menampilkan format NYATA dari situs

- Sebelum dialog terbuka, GUI menjalankan `yt-dlp -J` (simulated extraction,
  cap 20 dtk, mengikuti proxy & verify_tls + cookies dari config). Hasilnya
  difilter (buang mhtml/duplikat/cap 24 entri) dan ditambahkan sebagai section
  "Format lengkap dari situs" di bawah preset yang sudah ada.
- Id format terpilih (mis. `137+140`) diteruskan ke `--format` dengan fallback
  `/best` (`quality_args` arm baru) — tetap melewati guard karakter; preset
  lama tidak berubah perilakunya.
- Gagal fetch / timeout / yt-dlp tanpa JSON → dialog hanya berisi preset
  (perilaku ≤2.5.x, tanpa regresi). Jalur "Simpan Sebagai…" sengaja tetap
  statis (sudah dua dialog).
- Label dinamis dirender teks biasa (bukan markup Pango) — data dari halaman
  tidak pernah diinterpretasikan sebagai markup (anti injeksi).
- +3 unit test (parse JSON, garbage-tolerant, passthrough & guard format id).

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
