# Changelog

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/),
versi mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [2.10.2] - 2026-09-10

### Fixed

- Start/resume berulang kini idempotent: unduhan aktif tidak berubah menjadi
  antrean ketika slot penuh, dan tidak mengklaim worker kedua.
- Resume ketika pause masih menghentikan backend ditunda sampai supervisor lama
  selesai cleanup. Worker yang sedang berhenti tetap dihitung dalam batas slot;
  pause ulang/cancel membatalkan permintaan resume tertunda.
- Spawn subprocess dan publikasi PID dilakukan dalam satu lock, sehingga pause
  tidak kehilangan proses yang baru lahir. Respons resolver/tool yang terlambat
  tidak lagi menimpa status pause/cancel pada jalur yang diperbaiki.
- Promosi antrean membaca konfigurasi terbaru, bukan snapshot milik unduhan lama.
  Penghitungan percobaan juga bertambah ketika item antrean benar-benar dimulai.
- Shutdown mencegah start/promosi worker baru; flag lifecycle hanya disimpan di
  memori dan tidak masuk session.json.
- Jeda individual tersedia untuk item antrean.
- CI extension memasang Node.js 22 melalui `actions/setup-node`, bukan memasukkan
  `node-version` sebagai opsi checkout.

### Tests

- Regression test lifecycle untuk start berulang, deferred resume, pause/cancel,
  slot worker yang sedang berhenti, konfigurasi antrean terbaru, shutdown,
  serialisasi flag runtime, serta runner yang menerima item pause/cancel.

## [2.10.1] - 2026-09-10

### Fixed

- Resolver nama/ukuran file menggunakan proxy dari Pengaturan, termasuk SOCKS.
  Cache HTTP client mengikuti perubahan proxy/TLS; proxy invalid ditolak tanpa
  mencantumkan kredensial di pesan error.
- Argumen jaringan yt-dlp dipakai bersama oleh pengambilan format, unduhan
  YouTube, dan resolver universal. Opsi verifikasi TLS kini konsisten.
- Deduplikasi unduhan melakukan pengecekan dan insert dalam satu write-lock,
  sehingga request bersamaan tidak sama-sama memasukkan item identik.
- Probe dan pembacaan clipboard dipindahkan dari thread GTK ke subprocess Tokio,
  dibatasi 1 detik dan 2 KiB, tanpa polling bertumpuk. Proses yang macet dihentikan
  beserta process group-nya dan di-reap.
- Popup hasil pemindaian tidak lagi mengklaim “Terkirim” sebelum respons sukses.
  Respons gagal/kosong atau error transport mengaktifkan kembali tombol retry;
  badge background juga mencerminkan penolakan native host.
- Log sukses extension tidak lagi memuat URL unduhan yang mungkin bertoken.

### Tests

- Regression test Rust untuk proxy resolver, setelan jaringan yt-dlp,
  deduplikasi konkuren, serta timeout/batas output clipboard.
- Regression test extension memakai Node.js test runner dan mock API browser
  tanpa dependensi tambahan; dijalankan sebelum packaging extension di CI/rilis.

## [2.10.0] - 2026-09-07

Rilis perbaikan hasil review kode menyeluruh: 6 bug logika, 4 celah
privasi/keamanan, 4 masalah performa/UX, plus penjaga regresi baru.
Label butir (A/B/C/D) merujuk pada hasil review, bukan `CODE-REVIEW.md`.

### Security

- **Kredensial tidak lagi bisa masuk `session.json`** (B1) — dua lapis:
  1. `Cookie`, `Authorization`, dan `Proxy-Authorization` DIHAPUS dari
     `ipc::HEADER_ALLOWLIST`. Tidak ada pemakainya: extension hanya pernah
     mengirim `Referer`, dan cookie sudah punya jalur sendiri yang lebih aman
     (field `cookies`+`domain` → file Netscape per-domain 0600 →
     `--load-cookies`/`--cookies`/opsi per-URI `cookie`). Lewat header, cookie
     justru berakhir di argv proses (terbaca di `/proc/<pid>/cmdline`) dan ikut
     tertulis ke disk.
  2. `downloader::redact_for_persist` membuang header sensitif dari snapshot
     sebelum ditulis. Ini juga membersihkan `session.json` warisan ≤2.9.4 pada
     flush berikutnya — file itu tidak punya kedaluwarsa (hanya cap 200 entri),
     jadi tanpa redaksi kredensial lama bertahan selamanya.
- **`Config::config_dir()` tidak lagi jatuh ke `/tmp`** (B2) — fallback lama
  `/tmp/fast-dm` adalah path publik yang bisa di-pre-create user lain, padahal
  isinya `config.json`, `rpc.secret`, `cookies_*.txt`, dan `session.json`.
  Sekarang bertingkat: XDG config dir → `$HOME/.config/fast-dm` →
  `temp_dir/fast-dm-<euid>` (di-namespace per-UID). Inti logika dipisah ke
  `config_dir_from()` agar bisa diuji tanpa mengutak-atik `HOME`.
- **`config.json` kini ditulis 0600** — ia bisa memuat `proxy_url` berisi
  kredensial (`http://user:pass@host:port`), tapi satu-satunya file rahasia di
  config dir yang belum diperketat.
- **`apply_autostart()` gagal dengan pesan, bukan menulis ke `/tmp/autostart`**
  (B2) — `.desktop` di `/tmp` tidak akan pernah dibaca session manager mana pun.
- **`rpc.secret` dibuat atomik dengan `O_EXCL` + mode 0600 sejak lahir** (B3) —
  sebelumnya `fs::write` biasa: dua proses yang start bersamaan saat pertama
  kali bisa sama-sama menghasilkan secret dan saling menimpa, sehingga daemon
  yang lahir dari proses A tidak bisa di-probe proses B. Yang kalah balapan
  kini membaca secret milik pemenang.
- **`ipc::peer_uid_ok` memakai `geteuid()`** (A5) — menyamakan identitas dengan
  `config::validated_runtime_dir()`. Keduanya identik untuk proses non-setuid,
  tapi dua keputusan keamanan tidak boleh memakai identitas berbeda.
  (`cleanup_legacy_socket` sengaja tetap `getuid()` — ia merekonstruksi path
  warisan ≤2.2.5 yang memang dibentuk dari `getuid()`.)

### Fixed

- **v2.9.4 tidak bisa di-build sama sekali** — `native_host::setup` memakai
  `{registry}` (sebuah `PathBuf`) sebagai argumen inline di `format!` dan
  `tracing::warn!`, padahal `PathBuf` tidak mengimplementasi `Display`
  (E0277 di dua tempat: `new_origin_notice` dan `notify_new_extension_id`).
  Sekarang `registry.display()`, yang mencetak path apa adanya tanpa tanda
  kutip `{:?}` — jadi isi pesan dan test `notice_names_the_id_and_how_to_revoke`
  tidak berubah. Karena kesalahan ini menggagalkan kompilasi seluruh crate,
  angka "190 test" yang diklaim rilis 2.9.4 tidak pernah benar-benar dijalankan.
- **Skema URL berhuruf besar tidak lagi dirusak** (A1) —
  `normalize_url_input("HTTP://example.com/f.zip")` sebelumnya menghasilkan
  `https://HTTP://example.com/f.zip` dan gagal resolve dengan pesan yang tidak
  menjelaskan apa pun. Hanya `magnet:` yang di-lowercase; pencocokan
  `http://`/`https://`/`ftp://` bersifat case-sensitive. Kini semuanya
  case-insensitive, dengan casing URL asli tetap dipertahankan.
- **IPC tidak lagi mengaku sukses untuk unduhan yang ditolak** (A2) — handler
  `download` selalu membalas `success: true` karena `add_download` tetap
  mengembalikan id (dengan status `Error` di dalamnya). Extension lalu
  menampilkan badge ⬇ biru untuk `blob:`/`data:`/`file:` yang tidak akan
  pernah terunduh. Skema kini ditolak di boundary IPC; guard di dalam engine
  tetap ada untuk jalur GUI.
- **Pre-check ruang disk memakai satuan yang benar** (A3) — `statvfs(3)`
  menyatakan `f_bavail` dalam satuan `f_frsize`, bukan `f_bsize`. Kode lama
  mengalikan `blocks_available()` dengan `block_size()`. Di ext4/xfs/btrfs
  keduanya sama sehingga tidak terlihat; di NFS/FUSE bisa berbeda dan membuat
  unduhan lolos pre-check padahal disk tidak muat. Inti dipisah ke
  `available_bytes()` (murni, teruji) dengan fallback bila `f_frsize` 0 dan
  `saturating_mul` anti-overflow.
- **`#[allow(dead_code)]` yang tertinggal dihapus** (A6) — CHANGELOG 2.9.4
  sudah mengklaim tiga atribut di `gui/youtube_dialog.rs` hilang
  (`QualityOption`, `QUALITIES`, `show_quality_dialog`); yang benar-benar
  terhapus hanya dua.

### Changed

- **Unduhan http/ftp tidak lagi membayar 6 detik per unduhan saat daemon RPC
  tidak tersedia** (C1) — kegagalan `ensure_daemon` sebelumnya tidak diingat
  sama sekali, jadi bila `rpc_port` dipakai daemon aria2 asing setiap unduhan
  mengulang: probe gagal → spawn `aria2c` (lahir lalu mati karena bind gagal) →
  `wait_ready` 6 detik → fallback diam-diam. Kini kegagalan di-cache 60 detik
  (`daemon_gate_closed`, teruji) dengan pesan yang menyebut `rpc_port`, dan
  gerbangnya dibuka lagi begitu daemon terbukti siap ATAU user menyimpan
  Pengaturan (`update_config` → `reset_daemon_gate`) supaya pergantian
  `rpc_port` langsung dicoba.
- **Menutup jendela tidak lagi membekukan UI sampai ±9 detik** (C2) —
  `connect_close_request` memanggil `block_on(engine.shutdown())` di main
  thread GTK, padahal `shutdown_daemon` punya timeout berantai (forcePauseAll
  2 dtk → shutdown 2 dtk → forceShutdown 2 dtk → tunggu child/probe 3 dtk).
  Sekarang mesin tutup tiga tahap: konfirmasi → `Stop`; shutdown jalan di task
  lalu `win.close()`; `close_request` berikutnya melihat `close_done` →
  `Proceed`. Idempotent, dan `app.connect_shutdown` tetap jadi jaring pengaman.
- **`yt-dlp -J` di-spawn dengan `process_group(0)`** (C3) — menyamakan dengan
  aturan semua child lain (AGENTS.md §3). Tanpa group, saat timeout 20 dtk
  membuat future di-drop, `kill_on_drop` hanya menjangkau proses yt-dlp dan
  anak yang terlanjur lahir menjadi yatim.
- **Tombol "Pindai" di popup kini menampilkan kandidat hasil sniffing** (C4) —
  `detectVideos` tidak pernah membaca `dataset.fastdmMedia`, padahal justru di
  situlah `.m3u8`/`.mpd` hasil hook `fetch`/`XHR` paling berharga: URL itu
  tidak muncul di DOM sehingga sapuan `video`/`a[href]` tidak akan
  menemukannya. Sniffer mengumpulkan sampai 50 kandidat tetapi hanya satu
  (terbaru) yang pernah dipakai. Pembacaannya diekstrak ke
  `readSniffedCandidates()`; popup memakai seluruh daftar, tombol per-elemen
  media tetap memakai yang terbaru.
- **`Config::load()` → `Config::load_startup_snapshot()`** (D7) — nama lama
  mengundang pemanggil baru mengira ia membaca ulang dari disk setiap kali,
  padahal `OnceLock` hanya diisi sekali dan nilai yang benar setelah startup
  adalah `engine.get_config().await`.

### Added

- **Penjaga regresi: dua daftar ekstensi tidak bisa lagi melenceng diam-diam**
  (D5) — `DIRECT_FILE_EXTENSIONS` (Rust) dan `videoExtensions`/`fileExtensions`
  (`extension/background.js`) selama ini hanya dijaga komentar
  ("M2: SELARASKAN…"). Test `extension_intercept_list_is_covered` membaca
  `background.js` lewat `include_str!`, mem-parse kedua array, dan menuntut
  setiap ekstensi yang di-intercept browser dikenali `is_direct_file_url` —
  dengan pengecualian SADAR `.m3u8`/`.mpd` yang dikunci test terpisah
  (`hls_manifests_are_deliberately_not_direct_files`) agar manifest HLS/DASH
  tidak pernah dialihkan dari yt-dlp.
- 19 unit test baru (total crate 190 → 209): `config_dir_from` (3),
  `available_bytes` (3), gerbang daemon (3), redaksi kredensial + penjaga
  allow-list (6), `normalize_url_input` case-insensitive (2), dan penjaga
  daftar ekstensi (2).

### Catatan rilis

- Versi disamakan di `Cargo.toml`, `Cargo.lock`, dan
  `extension/manifest.json` (2.10.0) — `Cargo.lock` ikut diperbarui karena CI
  menjalankan `cargo build --release --locked`.
- Tidak ada perubahan `README.md`: seluruh butir di atas adalah perbaikan dan
  pengetatan, bukan fitur baru yang terlihat user.
- `cargo clippy --all-targets -- -D warnings` di CI SENGAJA masih advisory
  (`continue-on-error: true`). Menaikkannya jadi gate blocking tanpa bisa
  menjalankan clippy lebih dulu berisiko membuat CI merah; itu pekerjaan
  terpisah di mesin dev.

- Versi disamakan di `Cargo.toml`, `Cargo.lock`, dan
  `extension/manifest.json` (2.10.0) — `Cargo.lock` ikut diperbarui karena CI
  menjalankan `cargo build --release --locked`.
- Tidak ada perubahan `README.md`: seluruh butir di atas adalah perbaikan dan
  pengetatan, bukan fitur baru yang terlihat user.
- `cargo clippy --all-targets -- -D warnings` di CI SENGAJA masih advisory
  (`continue-on-error: true`). Menaikkannya jadi gate blocking tanpa bisa
  menjalankan clippy lebih dulu berisiko membuat CI merah; itu pekerjaan
  terpisah di mesin dev.

## [2.9.4] - 2026-09-07

### Security

- **Header dari extension kini disaring allow-list di boundary IPC**
  (`ipc::sanitize_headers`) — sebelumnya field `headers` pada pesan IPC
  diteruskan apa adanya ke argumen CLI aria2 (`--header=`) dan yt-dlp
  (`--add-header`), sehingga proses lokal mana pun dengan UID sama bisa
  menyuntikkan header arbitrer ke permintaan unduhan. Yang diterima sekarang
  hanya `Referer`, `Origin`, `Cookie`, `Authorization`, `Accept-Language`, dan
  `User-Agent` (dicocokkan case-insensitive, casing asli dipertahankan).
  Control char (`\r`, `\n`, `\0`, tab, DEL) dibuang dari nama maupun nilai
  sebagai lapisan kedua anti header-injection, whitespace di ujung di-trim,
  nilai dipotong di char boundary pada 8 KB, dan jumlah dibatasi 16 entri
  dengan urutan deterministik (tidak bergantung acakan `HashMap`). Tidak ada
  perubahan perilaku yang terlihat user: extension hanya pernah mengirim
  `Referer`. Menutup rekomendasi `CODE-REVIEW.md` §C3.
- **Validasi extension ID diperketat ke bentuk Chrome yang sebenarnya**
  (`native_host::setup::is_valid_extension_id`) — tepat 32 karakter dari
  himpunan `a`–`p`. Validasi lama ("≥20 karakter alfanumerik apa pun")
  membiarkan string arbitrer masuk `allowed_origins` manifest Native Messaging,
  dan tidak konsisten dengan `setup-browser.sh` yang sudah lama menolak apa pun
  di luar `^[a-p]{32}$`. Kedua penulis manifest itu kini setuju — penting
  karena keduanya menulis file yang sama. Menutup rekomendasi §C2.
- **`extension_ids.json` dibatasi 8 entri dengan eviksi LRU** — setiap ID
  terdaftar menjadi satu origin yang diizinkan memanggil native host, jadi
  tanpa batas proses lokal bisa memperbesar daftar origin (dan manifest) tanpa
  batas. ID yang di-register ulang digeser ke posisi paling baru agar yang
  masih aktif tidak ter-evict lebih dulu, dan entri warisan validasi lama yang
  tidak valid dibuang saat registry dibaca.
- **User kini diberi tahu saat sebuah extension ID baru diizinkan memanggil
  native host** (menutup sisa rekomendasi §C2 — konfirmasi user saat `register`)
  — aksi `register` bisa datang dari socket IPC (proses lokal mana pun dengan
  UID sama, setelah cek `SO_PEERCRED`) maupun dari native host, dan ID yang
  bentuknya valid akan diterima. `is_valid_extension_id` dan cap LRU membatasi
  seberapa jauh itu bisa pergi, tetapi tidak satu pun bisa membedakan
  extension sah dari proses lokal yang sedang memasang persistensi untuk dirinya
  sendiri. Hanya user yang bisa — jadi sekarang user diberi tahu: selalu lewat
  `tracing::warn!`, plus notifikasi desktop best-effort lewat `notify-send`.
  - Notifikasi hanya untuk origin yang BENAR-BENAR baru. `push_registered_id`
    kini mengembalikan apakah ID itu baru bagi registry, sehingga register ulang
    (retry `background.js`, browser restart) yang hanya me-refresh posisi LRU
    tidak memunculkan notifikasi berulang. ID bawaan `EXT_ID` dilewati:
    `make_origins` selalu memasangnya di posisi pertama apa pun isi registry,
    jadi register atas ID itu tidak memberi akses baru kepada siapa pun dan
    mengumumkannya hanya menjadi noise pada pemasangan normal.
  - `notify-send` (libnotify) dipakai, bukan notifikasi GTK: `register`
    diproses di process native host yang tidak punya koneksi display/GTK sama
    sekali, dan objek GTK tidak `Send` sehingga tidak bisa diserahkan ke task
    IPC. Spawn-nya memakai `std::process`, bukan `tokio::process`, karena
    `native_host::run()` adalah loop stdio sinkron tanpa runtime tokio —
    `tokio::process` di jalur itu akan panic. Kegagalannya tidak pernah
    menggagalkan register. `libnotify-bin`
    ditambahkan sebagai `Recommends` (bukan `Depends`) di `packaging/control`
    supaya tersedia pada pemasangan normal tanpa memaksa container/minimal
    install memasang stack libnotify demi fitur best-effort.
  - Pengumuman dilakukan tepat setelah registry ditulis, bukan setelah manifest:
    registry itulah sumber `allowed_origins` dan ikut dipakai `check_and_setup`
    saat start, jadi izinnya bertahan walaupun penulisan manifest kali ini gagal.
  - Anak proses `notify-send` di-reap di thread terpisah. Ia biasanya selesai
    dalam milidetik tetapi bisa menggantung bila daemon notifikasi macet,
    sedangkan pemanggilnya adalah loop pesan native host / accept IPC yang tidak
    boleh terblokir; `std::process::Child` yang di-drop TIDAK di-reap Rust, jadi
    tanpa `wait()` ia menjadi zombie selama proses induk hidup.

### Fixed

- **Server IPC tidak lagi mati permanen saat `accept()` gagal** — satu error
  transien (fd habis = `EMFILE`/`ENFILE`, `ENOBUFS`, `ECONNABORTED`) dulu
  dipropagasi keluar dari `ipc::start_server` lewat `?`, dan pemanggilnya hanya
  menulis log. Akibatnya native messaging putus untuk SELURUH sesi aplikasi:
  extension tidak pernah bisa mencapai GUI lagi sampai `fast-dm` di-restart,
  tanpa petunjuk apa pun di UI. Sekarang error dicatat lalu `accept` dicoba
  ulang dengan backoff eksponensial 50 ms → 2 dtk (di-cap), dan counter
  kegagalan di-reset begitu satu koneksi berhasil. Kegagalan `bind` dan
  `set_permissions` saat start tetap fatal seperti sebelumnya.

- **Badge ekstensi tidak lagi dihapus oleh timer milik badge sebelumnya** —
  setiap `showBadge` memasang `setTimeout` 3 detik yang tidak pernah dibatalkan,
  jadi urutan badge pending → badge hasil (atau dua unduhan berdekatan) membuat
  timeout dari badge LAMA menghapus badge yang baru dipasang lebih cepat dari
  seharusnya. Timer kini dilacak dan dibatalkan sebelum badge baru dipasang.

### Changed

- **Dead code dibersihkan** — `DownloadEngine::get_download()` dihapus (nol
  pemanggil di seluruh crate; `get_all_downloads()` yang dipakai GUI/IPC).
  `#[allow(dead_code)]` blanket pada `impl DownloadEngine` ikut dihapus karena
  hanya menyembunyikan dead code yang muncul di kemudian hari, bersama tiga
  `#[allow(dead_code)]` basi di `gui/youtube_dialog.rs` (`QualityOption`,
  `QUALITIES`, `show_quality_dialog` — ketiganya terpakai).
- Komentar yang menyesatkan diperbaiki: `native_host::run()` bukan "baca satu
  message, respond, exit" melainkan loop sampai EOF (bentuk yang juga melayani
  `chrome.runtime.connectNative`), dan provider CSS di `gui::window` dipasang
  display-wide — yang mencegahnya bocor ke aplikasi lain adalah prefix selector
  `.fast-dm-window` pada setiap rule, bukan cara provider dipasang.

- **Biaya `sniffer.js` di halaman tanpa media ditekan mendekati nol**
  (menutup rekomendasi `CODE-REVIEW.md` §B4 a–d) — script ini di-inject ke
  MAIN world setiap halaman yang dikunjungi, padahal mayoritas halaman tidak
  punya media sama sekali:
  - `persist()` di-batch per microtask dan hanya menulis bila kandidat
    benar-benar bertambah. Sebelumnya seluruh `Set` (hingga 50 URL) di-
    `JSON.stringify` ulang setiap kali satu URL lolos saringan — puluhan kali
    per burst mutasi. Microtask dipilih, bukan `requestAnimationFrame`, karena
    rAF di-throttle (bahkan dihentikan) di tab latar sehingga kandidat bisa
    tidak pernah terbaca oleh `content.js`.
  - `MutationObserver` memindai hanya node yang baru ditambahkan atau yang
    atribut `src`-nya berubah, bukan `querySelectorAll` atas SELURUH dokumen
    tiap burst 300 ms. Biaya kini sebanding dengan ukuran konten baru, bukan
    ukuran halaman; halaman berisi ribuan `a[href]` dulu membayar sapuan penuh
    berulang kali.
  - Halaman yang sampai 8 detik setelah `load` tidak punya kandidat sniffing
    maupun elemen media masuk mode _dormant_: callback observer tinggal
    memeriksa nama tag node baru. Tidur tidak permanen — sniffer bangun lagi
    (dan boleh tidur lagi) bila muncul `video`/`audio`/`source`, `<a href>`
    ber-ekstensi media, atau URL berubah (navigasi SPA).
    `<a href="…mp4">` yang berada DI DALAM subtree baru saat dormant memang
    tidak membangunkan sniffer; tombol "Pindai" di popup tetap menemukannya
    karena `detectVideos` pada `content.js` menyapu `a[href]` seluruh dokumen
    secara on-demand dan tidak bergantung pada sniffer.
  - `manifest.json`: kedua content script kini mengecualikan properti Google
    (`*://*.google.com/*`, `*://*.googleapis.com/*`, `*://*.gstatic.com/*`)
    untuk memangkas injeksi MAIN-world. `host_permissions` tetap `<all_urls>`
    karena memang diperlukan untuk cookies/referer. Intersep unduhan dan
    context menu tidak terpengaruh (keduanya API service worker, bukan content
    script), dan embed YouTube di halaman Google tetap mendapat overlay karena
    dokumen iframe-nya berasal dari `youtube.com`.
- **Config ekstensi pindah dari `chrome.storage.sync` ke
  `chrome.storage.local`** (menutup §B4e; L6 sebagian — sisi `storage.sync`
  selesai, sisi "timer badge hilang saat service worker di-suspend" belum) —
  config ini per-mesin (intersep, ambang ukuran, daftar ekstensi file), bukan
  preferensi yang perlu ikut ke perangkat lain, sedangkan `sync` berkuota ketat
  (8 KB/item, ±512 tulis/hari, di-throttle) yang bisa membuat "Simpan" di popup
  gagal tanpa pesan apa pun. Config lama dimigrasi satu arah saat service worker
  start lalu salinan di `sync` dihapus supaya tidak ada dua sumber kebenaran;
  bila tulis ke `local` gagal, salinan `sync` dibiarkan utuh agar config user
  tidak hilang.
- **Badge `…` selama unduhan sedang dikirim ke aplikasi** (menutup §B4f) —
  `sendToNative` menunggu hingga 25 detik sebelum timeout, dan sebelumnya badge
  baru muncul SETELAH native host menjawab. Selama cold-start GUI user tidak
  mendapat umpan balik apa pun dan wajar mengira kliknya tidak terdaftar. Badge
  pending kini ditahan sampai badge hasil (`⬇`/`!`) menggantikannya.

### Added

- 27 unit test baru: `ipc::accept_backoff` (4) dan `ipc::sanitize_headers`
  beserta helper `truncate_chars`/`strip_control` (11) — `ipc/mod.rs` naik dari
  0 ke 15 test; serta `native_host::setup` (12) untuk `is_valid_extension_id`,
  `push_registered_id`, dan `make_origins` termasuk penjaga agar
  `allowed_origins` tidak pernah memakai wildcard. Kedua modul itu sebelumnya
  tidak punya test sama sekali (total test crate 159 → 186).
- 4 unit test tambahan di `native_host::setup` untuk §C2 (total modul ini
  12 → 16): ID ter-evict yang mendaftar lagi dilaporkan sebagai baru, `EXT_ID`
  bawaan bukan origin baru (diperiksa dalam bentuk trim, bentuk mentah
  `include_str!` yang membawa newline, dan bentuk ber-padding spasi), ID valid
  lain adalah origin baru, dan teks notifikasi menyebut ID beserta berkas
  registry sebagai cara mencabut izin. Total test crate 186 → 190.

## [2.9.3] - 2026-09-07

### Fixed

- **Koneksi per server > 16 tidak lagi mematikan unduhan** — Pengaturan
  mengizinkan 1–32, tetapi `aria2c` menolak seluruh baris perintah bila
  `--max-connection-per-server` di luar 1–16. Nilai kini di-clamp lewat satu
  helper bersama (`aria2::conn_per_server`) yang dipakai jalur per-proses,
  `daemon_args`, dan opsi per-URI RPC; `--split` tetap memakai nilai penuh
  (aria2 tidak membatasinya).
- **Perubahan Pengaturan berlaku tanpa restart untuk jalur daemon RPC** —
  daemon hanya membaca `daemon_args` saat lahir (termasuk daemon yatim dari
  sesi app sebelumnya), sehingga koneksi/timeout/retry/proxy/TLS lama ikut
  terbawa. Kini setiap unduhan baru menyinkronkan opsi ke daemon hidup lewat
  dua panggilan `changeGlobalOption` terpisah — inti (limit kecepatan +
  `max-concurrent-downloads`) dan pelengkap best-effort — supaya satu kunci
  yang ditolak daemon tidak ikut membatalkan penerapan limit.
- **Opsi `addUri` sejajar dengan jalur CLI** — `max-connection-per-server`,
  `split`, `auto-file-renaming`, serta `all-proxy`/`check-certificate`
  sekarang dikirim per-URI. Sebelumnya nilai-nilai itu bergantung pada state
  global daemon, sehingga unduhan RPC bisa memakai proxy/verifikasi TLS/aturan
  penamaan file yang berbeda dari yang dipilih user.
- **Cookie dari ekstensi tidak lagi dianggap basi terlalu cepat** — ambang
  kesegaran file cookie disamakan dengan TTL yang benar-benar ditulis
  (`ipc::write_cookies_txt` = 24 jam); sebelumnya 2 jam, sehingga yt-dlp jatuh
  ke `--cookies-from-browser` yang sering gagal saat browser sedang berjalan
  dan unduhan login-protected ikut gagal. GC 7 hari tetap berlaku.
- **Ekstensi: penanda anti-loop `selfInitiated` kedaluwarsa otomatis (60 dtk)**
  — dulu entri hanya dihapus saat event `downloads.onCreated` untuk URL yang
  sama tiba; bila download fallback tidak pernah terbentuk (dialog "Simpan
  sebagai" ditutup, URL ditolak Chrome, dll.) entri tertinggal selamanya di
  service worker dan membuat unduhan ULANG URL yang sama diam-diam dilewatkan.
- **`setup-browser.sh` tidak lagi memutus extension unpacked** — script menulis
  ulang manifest hanya dengan ID packed, menghapus origin extension dev yang
  sudah di-register aplikasi (`extension_ids.json`). Kini ID yang terdaftar
  ikut digabung, EXT_ID kosong/tidak valid ditolak (dulu diam-diam menulis
  `chrome-extension:///` dan native messaging mati tanpa pesan), manifest hanya
  ditulis untuk profil browser yang benar-benar ada (tidak lagi membuat ±13
  folder sampah di `~/.config`, sejalan dengan M8 di sisi Rust), dan
  `XDG_CONFIG_HOME` dihormati.
  - **`setup-browser.sh` kembali bisa dijalankan** — perombakan di atas
    kehilangan kurung kurawal penutup fungsi `write_manifest()`, sehingga bash
    menolak seluruh script (`syntax error: unexpected end of file`) sebelum
    satu manifest pun ditulis. Diverifikasi dengan `bash -n` dan uji jalan di
    `XDG_CONFIG_HOME` sementara (origin packed + registry tergabung, profil
    yang tidak ada dilewati, EXT_ID tidak valid / tanpa profil → exit 1).

- **Crate kembali dapat dikompilasi** — commit sebelumnya kehilangan dua baris
  di accept loop IPC (`tokio::spawn` + `stream.into_split()`), menyisakan blok
  `close_request` yang terduplikasi dan terpotong di `gui/window.rs`, serta
  satu karakter `/` nyasar di test `youtube.rs`. Ketiganya membuat
  `cargo build`/`cargo test` gagal parse.
- `cargo fmt --all -- --check` (gate CI) kembali hijau: indentasi dan baris
  kosong nyasar di `config.rs`, `downloader/mod.rs`, `aria2_rpc.rs`,
  `gui/window.rs` dirapikan. Tidak ada perubahan perilaku.

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
