# Analisis Kode & Rekomendasi Optimasi — Fast-DM v2.2.5

> Ditulis 2026-09-02. Hasil pembacaan penuh seluruh source (±8.000 baris: Rust, JS extension, shell, CI).
> Referensi format `file:line`.

---

## 0. Status Perbaikan (v2.3.0 — 2026-09-02, branch `arena/01a0626b-fast-dm`)

| Item | Status | Catatan |
|---|---|---|
| K1 socket /tmp | ✅ Fixed | `Config::ipc_socket_path()` → XDG_RUNTIME_DIR (validasi owner+0700) / fallback `~/.config/fast-dm/run`; `SO_PEERCRED` uid-check di accept; socket lama dibersihkan saat start |
| K2 wildcard build.sh | ✅ Fixed | `build.sh` + `build.patch` + `ci/build.yml.example` dihapus; packaging resmi = `packaging/build-deb.sh` |
| K3 aria2 input di /tmp | ✅ Fixed | `Config::aria2_input_dir()` privat, file 0600, GC sisa crash saat engine start |
| K4 kill hanya parent | ✅ Fixed | child di-spawn `process_group(0)`; `kill_child_pid`=killpg SIGTERM (pause/resume-friendly), `kill_child_group_hard`=SIGKILL group (cancel); jalur tutup-window ikut pakai helper |
| K5 README vs kode | ✅ Fixed | `Config.auto_resume` (default ON) + `resume_restored()` — auto-resume nyata, toggle di Settings; README disinkronkan |
| M2 daftar ekstensi | ✅ Fixed | daftar direct-file diselaraskan (+tests) |
| M3 limit dibagi statis | ✅ Fixed | `resolve_speed_limit(total, live_share)` adaptif saat start; promosi membawa config mentah (anti double-division). Batas proses berjalan tidak di-recalc (butuh RPC — roadmap B2) |
| M5 session korup diam-diam | ✅ Fixed | format `{version:1,downloads}` + fallback legacy array + backup `*.corrupt-<ts>` + tests |
| M6 regex YouTube kaku | ✅ Fixed | host+path based, `/live/ /embed/ /v/`, `v=` posisi berapa pun, tolak host mirip |
| M7 cookie TTL 1 tahun | ✅ Fixed | TTL 24 jam + GC >7 hari saat start |
| M8 sampah folder browser | ✅ Fixed | manifest hanya ditulis bila profil browser ada |
| M9 leak listener overlay | ✅ Fixed | AbortController per-attach di content.js |
| M10 error_msg = info | ✅ Fixed | field baru `status_detail` + `.info-label` biru |
| M11 skema tak divalidasi | ✅ Fixed | `add_download` tolak non-http(s)/ftp dengan pesan jelas |
| L1 LICENSE/CHANGELOG hilang | ✅ Fixed | keduanya dibuat |
| L3 drift packaging | ✅ Fixed | file basi dihapus; versi Cargo.toml==manifest.json==2.3.0 |
| L4 FIFO tak deterministik | ✅ Fixed | `created` milidetik + kunci pembanding `id` |
| L7 timeout sendToNative | ✅ Fixed | 25 s + pesan |
| M1 tokio::process refactor | ✅ v2.3.1 | `tokio::process` + `ChildLines` + ticker 500ms + wait paus bounded 30s + kill_on_drop; panic `rt.block_on` di `universal.rs` ikut dibasmi |
| M4 dialog nested loop | ⏭️ Not started | ubah ke async pattern — berisiko tanpa test GUI |
| L2 AGENTS.md usang | ✅ Fixed | ditulis ulang v0.2.0 sesuai arsitektur nyata |
| L10 CI tanpa clippy/audit | ⚠️ Partial | komentar basi dibersihkan; step clippy/deny belum (butuh keputusan policy) |
| D-x (fitur), B-x (arsitektur RPC) | 📋 Roadmap | lihat §5 dokumen ini |

---

---

## 1. Gambaran Besar

Fast-DM adalah **Download Manager ala IDM untuk Linux** yang tidak mengimplementasikan
protokol HTTP download sendiri, melainkan menjadi **orkestrator** dari dua tool CLI ternama:

| Komponen | Teknologi | Peran |
|---|---|---|
| Core app | Rust 2021, Tokio, GTK4 (`gtk4 0.9`, feature `v4_12`) | Scheduler, state, GUI, IPC |
| Accelerator file langsung | **aria2c** (subprocess CLI) | Multi-koneksi, resume, segmented download |
| Resolver situs video | **yt-dlp** (subprocess CLI) | YouTube + 1800+ situs, HLS/DASH, ekstraksi audio |
| Integrasi browser | Ekstensi Manifest V3 + **Chrome Native Messaging** | Intercept download, overlay tombol ⚡, kirim cookies/referer |
| Packaging | `.deb` (dpkg-deb), zip extension, GitHub Actions CI | Distribusi |

Satu crate punya dua target: library `fast_dm` (di-import integration test) dan
binary `fast-dm` dengan dispatch mode via flag:

```
fast-dm            → GUI mode (GTK4 + Tokio + IPC server)
fast-dm --native   → Native Messaging Host (stdio JSON, dipakai browser)
```

## 2. Cara Kerja (Alur Data Lengkap)

### 2.1 Startup & wiring

1. `main.rs` — init `tracing` ke **stderr** (stdout dicuri protokol native messaging),
   panggil `native_host::setup::check_and_setup()` (menulis/validasi manifest NMH ke
   folder config ±13 browser Chromium-based), lalu dispatch mode.
2. `app.rs` — membangun `tokio::runtime` multi-thread (2 worker) yang **di-leak
   `Box::leak`** menjadi `'static` (`AppInit::try_new`, error dipropagasi, bukan panic),
   membuat `Application` GTK4 dengan app-id unik. Launch kedua dengan app-id sama
   hanya melakukan `window.present()` (single-instance, B2).
3. Saat `activate`: buat channel `mpsc::unbounded<DownloadEvent>`, `DownloadEngine::new`,
   lalu spawn **thread `fastdm-ipc`** yang menjalankan Unix-socket server
   (`/tmp/fast-dm-<uid>.sock`, mode 0600), dan `gui::window::build_window`.

### 2.2 Engine (`src/downloader/mod.rs`)

- State: `HashMap<id, Arc<Mutex<DownloadInfo>>>` di belakang `RwLock`, plus
  `AtomicBool dirty` untuk persistensi.
- `add_download()` → sanitize filename, **dedup** (url+dir+file yang masih live →
  kembalikan id lama, cegah dua aria2 menulis file sama), simpan, `auto_start`.
- `start_download()` → **satu write-lock** menghitung slot aktif & mengklaim status
  `Resolving` sekaligus (anti double-spawn, B3). Slot penuh → `Queued`.
- `spawn_supervised()` memilih jalur per URL:
  - `is_youtube_url()` → **yt-dlp** (`youtube.rs`) dengan dialog kualitas;
  - `is_direct_file_url()` (ekstensi .mp4/.zip/.pdf/… di PATH, fragment di-strip) →
    **aria2** langsung;
  - sisanya → **yt-dlp sebagai "resolver universal"** (`universal.rs`, ala IDM Grabber);
    kalau gagal → fallback ke aria2 (kecuali yt-dlp tidak terinstall → pesan jelas).
- Saat task selesai (sukses/gagal/cancel) → tandai dirty → `promote_next()`:
  ambil `Queued` tertua bila slot kosong — juga diklaim atomik dalam write-lock.
- **Pause/Resume/Cancel**: `kill_child_pid()` mengirim **SIGTERM** (aria2 sempat menulis
  `.aria2` control file → resumable; SIGKILL hanya saat Cancel).
- **Session persist**: flusher tokio tiap ≤2 detik menulis `session.json`
  (tmp+rename atomic, maks 200 entri terbaru). Saat restart, `Downloading/Resolving/Queued`
  → diubah `Paused` dengan `pid=None` (restore manual oleh user).

### 2.3 Runner aria2 (`aria2.rs`) — pipeline paling kaya

1. **Resolve** (`resolve_filename`): `GET Range: bytes=0-0` (fallback HEAD) pakai
   `reqwest::Client` **dibagi global** (OnceLock per verify_tls). Hasil:
   - tolak HTML / non-2xx (mencegah "file .php" berisi halaman web);
   - nama dari `Content-Disposition` (RFC 5987 `filename*=UTF-8''…` → quoted → unquoted);
   - fallback nama dari **URL final setelah redirect**;
   - `total_size` dari `Content-Range`/`Content-Length`;
   - koreksi ekstensi: nama `.php/.html/.do/…` yang ternyata `video/mp4` → ganti ekstensi;
     nama tanpa ekstensi → tambahkan dari content-type.
2. **Pre-check disk** via `statvfs` (gagal cek → jangan blokir).
3. **Spawn `aria2c`** lewat `std::process::Command` dalam `spawn_blocking`:
   URL ditulis ke **input-file** `/tmp/fast-dm/<id>.txt` (aman untuk URL panjang),
   semua opsi lain via argumen CLI (`--max-connection-per-server`, `--split`,
   `--min-split-size=1M`, `--continue`, `--allow-overwrite`/`--auto-file-renaming`
   mengikuti config, header custom anti-CRLF-injection, `--load-cookies=<per-domain>`,
   `--check-certificate` sesuai config).
4. **Parsing stdout**: regex dual-format (raw bytes + human `KiB/MiB`), **throttle 5 fps**;
   stderr dibaca thread terpisah (buffer 64KB, cap 16KB) — anti deadlock pipe.
5. Limit kecepatan user (total) **dibagi `max_concurrent`** per proses
   (`per_process_speed_limit`) karena tiap download = proses aria2c terpisah.

### 2.4 Runner yt-dlp (`youtube.rs` + `universal.rs`)

- Argumen: format per kualitas (`1080p` → `bestvideo[height<=1080]+bestaudio/…`),
  audio m4a/mp3, `--merge-output-format mp4`, `--embed-thumbnail/metadata` (YouTube saja),
  output template dengan escape `%`→`%%` (B6) dan fallback `%(title)s.%(ext)s`.
- **Cookies**: prioritas file `cookies_<host>.txt` kiriman extension bila **fresh < 2 jam**
  (`is_fresh_cookie_file`), dengan **walk-up domain** (`cdn.site.com` → `site.com`);
  fallback `--cookies-from-browser <browser>` — browser dideteksi dari
  `xdg-settings default-web-browser` (B17, di-cache LazyLock).
- Availability check `yt-dlp --version` di-cache OnceLock per sesi (B10).
- Progress: regex `[download] x% of ~SIZE at SPEED ETA`, merge → progress 99% + pesan
  "Merging…"; status terminal + event sama seperti aria2 (helper `run_ytdlp` dipakai bersama).

### 2.5 IPC & Native Messaging (`ipc/`, `native_host/`)

```
Extension (SW) ──sendNativeMessage──> fast-dm --native ──1 baris JSON──> Unix socket ──> GUI engine
        ▲                                │ (socket tak ada → spawn GUI,           │
        └────────── respons JSON ────────┘  poll socket maks ±15 dtk, lalu retry) ┘
```

- Protokol socket: **1 baris JSON masuk, 1 baris JSON keluar**, cap 1 MB.
- Aksi: `download` (sekalian **menulis `cookies_<host>.txt` mode 0600 sebelum spawn** —
  B7), `ping`, `pause`, `resume`, `cancel`, `list`, `register`.
- Single-instance server: bila socket sudah connect → skip bind (instance kedua tidak
  merebut koneksi browser).
- NMH: loop stdin length-prefixed (cap 1 MB), respons, exit saat EOF. GUI di-spawn
  `process_group(0)` + **stdio null** (B4: stdout GUI tidak boleh bocor ke pipe browser).
- `setup.rs`: manifest `com.fastdm.native.json` ditulis ke ±13 lokasi browser + profil
  Ice/Helium + glob `~/.config/*/Default`; **`allowed_origins` = EXT_ID tetap (packed via
  `key` di manifest + file `EXT_ID`) + daftar ID unpacked yang pernah `register`**
  (persisten di `extension_ids.json`) — manifest hanya ditulis ulang bila konten berubah.
  Dev mode: `resolve_native_path` membuat wrapper `fast-dm-native` shell di
  `target/{debug,release}/` agar NMH bisa dites tanpa install `.deb` (B16).

### 2.6 GUI GTK4 (`gui/`)

- Tema Catppuccin Mocha via CSS yang **di-scope class `.fast-dm-window`** (termasuk dialog,
  B2); list `ListBox` berisi kartu per download (`download_row.rs`) dengan badge status,
  progress, size/speed/ETA, baris error, dan 6 tombol (selalu tampil, hanya di-disable —
  layout tidak melompat, B4).
- Event loop: `while event_rx.recv()` di main-context → update row; stats
  (Aktif/Antri/Total + agregat speed) di-refresh dengan throttle 500 ms; seed ulang row
  dari session restore.
- Notifikasi desktop (`gio::Notification`) saat selesai/gagal.
- Toolbar: input URL (+ Enter), Unduh Semua→"Jeda/Lanjut Semua" sinkron state nyata,
  Bersihkan Selesai, Pengaturan (dialog dengan **validasi inline** A2, folder picker C1);
  `Ctrl+L` fokus URL; **drag & drop URL** multi-item C2; **Enter-close protection**:
  `connect_close_request` mengkonfirmasi bila ada unduhan aktif lalu SIGTERM semua child.
- Dialog kualitas YouTube (`youtube_dialog.rs`) dan dialog settings memakai pola
  **nested main loop** `while dialog.is_visible() { main_context.iteration(true) }`
  (pola阻塞 yang disengaja agar API tetap sinkron).

### 2.7 Ekstensi Browser (`extension/`)

- `manifest.json`: MV3, `key` dipin → ID unpacked stabil (`EXT_ID` di repo = ID yang sama),
  permission: `downloads, nativeMessaging, storage, activeTab, contextMenus, cookies,
  clipboardRead`, host `<all_urls>`.
- `background.js` (service worker):
  1. **auto-register extension id** ke NMH (sekali per ID; retry on startup/install);
  2. `chrome.downloads.onCreated` → **intersep**: filter (exclude pattern, ekstensi file,
     mime, **skip host YouTube**) → `downloads.cancel+erase` → `sendDownload()` ke app;
     bila app mati → **fallback** `chrome.downloads.download` normal dengan guard
     `selfInitiated` anti-loop tak berujung;
  3. context menu "⚡ Unduh" untuk link/video/image (menyertakan `Referer`);
  4. `sendDownload` mengambil **cookies via `chrome.cookies.getAll`** (bisa HttpOnly —
     sengaja tidak pakai `document.cookie`, B15) + filename dari URL, lalu kirim ke app;
  5. badge ⬇/! + pesan error eksplisit (B14).
- `sniffer.js` (MAIN world, `document_start`): **hook `fetch` & `XHR.open`** + scan
  `<video>/<audio>/<source>/<a href>` → kumpulkan URL media (`.m3u8/.mpd/.mp4/…`,
  resolve relatif→absolut, cap 50) ke `documentElement.dataset.fastdmMedia` (JSON)
  — satu-satunya jembatan MAIN→ISOLATED world.
- `content.js` (ISOLATED): overlay ⚡ IDM-like di player YouTube (watch/shorts/embed,
  dropdown 9 kualitas, auto-detect navigasi SPA via MutationObserver debounce 400 ms,
  listener klik-luar dipasang sekali anti-leak); di situs non-YouTube: tombol ⚡ per
  `<video>`, prioritas target: **kandidat sniffer > src langsung (bukan `.php/.html`) >
  URL halaman** — menyelamatkan wrapper page yang ternyata media.
- `popup.js/html/css`: status koneksi (ping), kirim URL (auto-paste clipboard),
  scan video aktif di tab, toggle intersep/enabled (sync ke `chrome.storage.sync`).

### 2.8 Persistensi & Config

- `~/.config/fast-dm/config.json` — 11 field, fallback berjenjang untuk `download_dir`,
  tulis atomik, **rusak → pakai default + warning (tidak diam-diam reset)**;
  `#[serde(default)]` untuk forward/backward compat.
- `cookies_<host>.txt` per-domain (sanitasi karakter, `www.` strip + lowercase **sebelum**
  strip, lookup walk-up ke domain induk).
- `session.json` — snapshot maks 200 entri terbaru, toleran rusak (`unwrap_or_default`).
- `extension_ids.json` — daftar NMH `allowed_origins` tambahan.

### 2.9 CI & Packaging

- `.github/workflows/ci.yml`: fmt check (blocking) → `cargo test --no-fail-fast` (±106 test)
  → `build --release --locked` → smoke `--version/--help` → artifact binary; job terpisah
  zip extension dan `.deb` via `packaging/build-deb.sh` (versi dibaca dari Cargo.toml).
  `deb`+`rpm` deps: `libgtk-4-1 (>=4.12), aria2, yt-dlp, ffmpeg, xdg-utils`.
- Catatan: `build.sh` (akar repo) adalah skrip build+deb **lama/duplikat** yang sudah
  tidak sinkron — lihat tabel risiko di bawah.

---

## 3. Kelebihan

**Arsitektur & rekayasa**
1. **Tidak reinvent-the-wheel**: download berat didelegasikan ke aria2c/yt-dlp yang sudah
   tahan banting (resume, multi-connection, 1800+ extractor, HLS/DASH merge). Aplikasi
   fokus ke apa yang tool CLI tidak punya: GUI, antrean, integrasi browser, persistensi.
2. **Fallback berjenjang yang dipikir matang**: yt-dlp gagal → aria2; yt-dlp tidak ada →
   pesan jelas tanpa fallback; cookie file per-domain → cookies browser; resolve gagal →
   lanjut nama dari URL; `statvfs` gagal → jangan blokir; config korup → default + warning.
3. **Manajemen konkurensi benar**: klaim slot & promote_next atomik dalam satu write-lock
   (B3), guard status "Cancelled/Paused" sebelum spawn (mencegah download hantu),
   lock-ordering map→mutex dikontrak dokumentasi (B13), fix deadlock RwLock non-reentrant
   didokumentasikan eksplisit.
4. **Robustness parsing output**: dual-format regex aria2 (bug lama "progress tidak pernah
   update" sudah diperbaiki + regression test), stderr di thread terpisah + cap 16KB
   (anti deadlock pipe), throttle 5 fps UI.
5. **Proses mati dengan sopan**: SIGTERM agar `.aria2` control file tertulis (B8),
   reap child anti-zombie, GUI spawn detached stdio-null (B4), prerm bersihkan socket.

**Keamanan (di atas rata-rata proyek hobi)**
6. Cookies & file sensitif mode **0600**; strip `\r\n` header anti injeksi; sanitasi
   filename + proteksi path traversal; cap pesan 1 MB di NMH **dan** IPC (anti OOM);
   validasi extension ID untuk `register`; `verify_tls` default ON (opt-out eksplisit);
   tolak HTML yang menyamar jadi file.
7. `allowed_origins` NMH **tanpa wildcard** di jalur utama (EXT_ID pin + registry
   register) — sengaja dicatat di komentar setup-browser.sh.

**Integrasi browser**
8. Sniffer MAIN-world (hook fetch/XHR) + jembatan dataset antar-world — teknik yang
   benar; overlay SPA-aware; HttpOnly cookie via `chrome.cookies` (bukan `document.cookie`).
9. Auto-register extension id + fallback intersep (cancel → kirim ke app → kalau app mati,
   Chrome lanjut download normal, `selfInitiated` anti-loop).

**Kualitas kode & DX**
10. ±106 unit+integration test untuk fungsi murni (parser, sanitizer, walk-up cookies,
    roundtrip serde, char-boundary truncation UTF-8), test tanpa dep berat (stdlib-only).
11. Komentar "mengapa" (bukan "apa") dengan kode bug (B1–B20) — jejak audit yang jelas;
    error init dipropagasi (bukan panic) `try_new` + signature-lock regression test.
12. Profil release agresif (LTO, codegen-units=1, strip, panic=abort); CI terstruktur +
    concurrency cancel; `.deb` + zip extension siap-pakai; dev-mode NMH wrapper otomatis.

---

## 4. Kekurangan & Risiko

### Kritis / Tinggi

| # | Isu | Detail | Lokasi |
|---|---|---|---|
| K1 | **Socket IPC di `/tmp` publik** | `/tmp/fast-dm-<uid>.sock` — user lain bisa **pre-create socket** di path itu; app melihat "connect OK" → skip server; extension mengirim **URL + cookies** ke socket milik attacker (pencurian kredensial). Solusi: `XDG_RUNTIME_DIR` (0700) atau verifikasi `SO_PEERCRED`+`getuid` di accept. | `ipc/mod.rs:36`, `native_host/mod.rs:105` |
| K2 | **`build.sh` postinst = wildcard `chrome-extension://*/*`** | Deb hasil `build.sh` mengizinkan **ekstensi apa pun** memanggil NMH → kirim aksi `download`/`list`/tulis cookies. (Jalur CI `packaging/build-deb.sh` aman, tapi dua jalur build yang bertentangan = jebakan.) Seharusnya hanya satu skrip build. | `build.sh:155` vs `setup-browser.sh:8` |
| K3 | **URL (sering bertoken) ditulis ke `/tmp/fast-dm/<id>.txt`** | Dir dibuat tanpa mode eksplisit → bergantung umask; file input-file berisi signed URL tertinggal bila app crash (tidak dibersihkan saat start). Solusi: dir `0700` under `XDG_RUNTIME_DIR`/`Config::config_dir()`, atau kirim URL via argumen, + GC file yatim saat start. | `aria2.rs:113-121` |
| K4 | **Kill hanya ke proses langsung** | `kill_child_pid` men-SIGTERM child (yt-dlp) tapi **ffmpeg cucu tidak ikut** — yt-dlp spawn ffmpeg untuk merge; pause/cancel bisa menyisakan ffmpeg yatim yang menulis file. Solusi: spawn child dengan `process_group(0)` lalu `killpg`. | `mod.rs:468` |
| K5 | **README vs kode: "otomatis di-resume saat restart" tidak benar** | Restore men-set `Paused` (`mod.rs:34`); tidak ada auto-start → user harus klik (atau "Lanjut Semua"). Fix salah satu: implementasikan auto-resume (opsi config) atau perbaiki klaim. | `README.md:15` |

### Sedang

| # | Isu | Detail | Lokasi |
|---|---|---|---|
| M1 | **1 blocking thread + 1 stderr thread per download**, `rt.block_on` per baris stdout | `spawn_blocking` + `std::process::Command` + sinkron; tiap baris output memicu block_on cek status. Jalan di 2-worker runtime + max 10 concurrent, tapi boros; arsitektur ideal: `tokio::process` + `AsyncBufRead` event-driven (lihat O2). | `aria2.rs`, `youtube.rs` |
| M2 | **Daftar ekstensi `is_direct_file_url` ≠ daftar extension-intercept** | `.exe/.msi/.dmg/.bz2/.doc(x)/.xls(x)` diintersep extension tapi tidak ada di daftar Rust → dibuang 1–3 dtk mencoba yt-dlp dulu, baru fallback aria2. | `mod.rs:392` vs `background.js` |
| M3 | **Limit kecepatan dibagi rata `max_concurrent` permanen** | 1 unduhan aktif hanya dapat `limit/3` — tidak memanfaatkan bandwidth; solusi benar: aria2 daemon JSON-RPC (satu `--max-overall-download-limit` global) atau hitung per **jumlah aktif saat ini**. | `aria2.rs per_process_speed_limit` |
| M4 | **Nested main-loop dialog sinkron** | `while dialog.is_visible() { iteration(true) }` — rapuh (app quit, dialog ditutup paksa), memblokir interaksi window lain; drag-drop multi-URL memunculkan dialog beruntun. Pola GTK4 yang benar: async dialog + callback. | `window.rs:890`, `youtube_dialog.rs:193` |
| M5 | **`session.json` korup → semua riwayat hilang diam-diam** | `unwrap_or_default()` tanpa backup/versi skema. | `mod.rs load_session` |
| M6 | **Deteksi YouTube terlalu kaku** | Regex menuntut `watch?v=` sebagai param pertama → `watch?app=desktop&v=…`, `/live/<id>`, `/embed/` (di GUI) tidak terdeteksi (content script memang handle embed, tapi jalur GUI/URL-typing miss → yt-dlp tetap berhasil via universal, hanya dialog kualitas tidak muncul). | `youtube.rs:9-12` |
| M7 | **Cookies lama 1 tahun di disk + tanpa cleanup** | `expires = now + 365d` untuk semua cookie; file `cookies_*.txt` tak pernah dihapus (freshness 2 jam hanya memengaruhi preferensi yt-dlp, bukan penghapusan). | `ipc/mod.rs:223` |
| M8 | **Manifest ditulis ke ±13 folder browser meski browser tidak terpasang** | `check_and_setup` `create_dir_all` buta → polusi `~/.config`. Seharusnya hanya bila folder browser ada. | `setup.rs check_and_setup/get_all_nmh_dirs` |
| M9 | **Leak listener overlay YouTube** | `attachOverlay` men-tambah `mouseenter/mouseleave` ke elemen player yang sama di tiap ganti video SPA (overlay lama di-`remove()` tapi listener di parent tidak) — menumpuk perlahan. | `content.js:485-505` |
| M10 | **`error_msg` dipakai sebagai channel info** | "Merging video + audio…" lewat `error_msg` → UI menampilkan baris error merah untuk proses normal; dan di-clear oleh progress berikutnya — semantik tercampur. | `youtube.rs` + `download_row.rs` |
| M11 | **Ekstensi `file://`/URL asing ke engine tanpa validasi skema** | `add_download` menerima string apa pun; `blob:` difilter extension tapi aksi IPC `download` langsung bisa berisi skema aneh → aria2 error lambat, bukan penolakan cepat. | `ipc/mod.rs handle_message` |

### Rendah / Polishing

| # | Isu | Lokasi |
|---|---|---|
| L1 | README link `LICENSE` & `CHANGELOG.md` **404** (dua file itu tidak ada di repo padahal badge MIT diklaim) | root |
| L2 | `AGENTS.md` usang: menggambarkan `src/aria2/`, `src/hls/`, JSON-RPC WebSocket — tidak ada satu pun di kode nyata; menyesatkan contributor/agent AI | `AGENTS.md` |
| L3 | Duplikasi & drift packaging: `build.sh` hardcode `2.1.3`, postinst echo "v2.1.0", vs Cargo `2.2.5`; `ci/build.yml.example` duplikat workflow; `build.patch` nyasar di repo; typo `.gitignore` `ekstension/` | `build.sh:7,229`, `.gitignore` |
| L4 | Order antrian tidak deterministik pada timestamp sama (`created` presisi detik, iterasi HashMap) | `mod.rs promote_next` |
| L5 | `retry_count` engine hanya di-increment; tidak ada auto-retry/backoff di level aplikasi (hanya aria2 `--max-tries` internal; setelah error total → status Error, manual "Ulangi") | `mod.rs` |
| L6 | `chrome.storage.sync` (kuota kecil, throttle) untuk config yang tidak perlu tersinkron antar perangkat → `local` lebih pas; badge `setTimeout` di SW bisa mati sebelum ke-sweep | `background.js` |
| L7 | `sendToNative` tanpa timeout JS (cold-start GUI ±15 dtk user lihat "tidak terjadi apa-apa") | `background.js` |
| L8 | Context-menu video tidak memanfaatkan kandidat sniffer → blob:/MSE `srcUrl` dikirim apa adanya dan gagal | `background.js onClicked` vs `content.js` |
| L9 | Tidak ada penghapusan entri lama yang **Completed** secara otomatis dari session selain cap 200; tidak ada tombol "hapus semua selesai+hapus file" | `mod.rs flush_session` |
| L10 | CI tanpa `cargo clippy`, `cargo audit`/`cargo deny`, tanpa job test extension (`web-ext lint`/vitest), komentar "97 unit test" padahal 106 | `.github/workflows/ci.yml` |
| L11 | Tidak ada skema/versi di `session.json`/`config.json` untuk migrasi | `config.rs`, `types.rs` |
| L12 | `resolve_client`: jika `build()` gagal sekali, tidak di-cache → retry builder tiap download (trivial tapi ada); OnceLock per verify_tls hanya men-cache satu pasangan | `aria2.rs:399` |

---

## 5. Yang Dapat Dioptimalkan (Rekomendasi)

### A. Quick wins (risiko rendah, dampak nyata) — bisa dikerjakan < 1 hari

1. **Pindahkan socket & file temp ke `XDG_RUNTIME_DIR`** (fix K1+K3 sekaligus):
   `let dir = dirs::runtime_dir().unwrap_or(/tmp)` → buat `0700`; tetap fallback
   `/tmp/fast-dm-<uid>` bila env kosong; validasi `SO_PEERCRED` (`nix::sys::socket::getsockopt`)
   di `accept()` dan tolak peer dengan uid berbeda. Bersihkan `*.txt` yatim saat start.
2. **`killpg` saat pause/cancel** (K4): spawn aria2/yt-dlp dengan
   `.process_group(0)` (std sudah support, sudah dipakai `native_host/mod.rs:120`),
   `kill_child_pid` → `nix::sys::signal::killpg(pid, SIGTERM)`; fallback ke pid tunggal
   bila `ESRCH`.
3. **Satukan daftar ekstensi direct-file** (M2): tambah `.exe .msi .dmg .bz2 .tbz2 .txz
   .doc .docx .xls .xlsx .ppt .pptx .epub .mobi .bin .img .dmg` ke `EXTENSIONS`
   `mod.rs:392` (atau konstanta tunggal yang dibaca dua tempat + test konsistensi).
4. **Perbaiki klaim auto-resume** (K5): tambah `Config.auto_resume: bool` (default true)
   → di `DownloadEngine::new`, entri `Paused` hasil restore langsung `start_download`
   (dengan jeda kecil agar tidak membanjiri); kalau tidak mau, ubah README.
5. **Backup `session.json` korup** (M5): simpan sebagai `session.json.bak` + log,
   dan tambah field `"_version": 1` di payload.
6. **Hanya tulis manifest bila folder browsernya ada** (M8): cek `dir.parent()`
   (folder browser) `is_dir()` sebelum `create_dir_all` di `check_and_setup`/`write_manifests`.
7. **TTL & cleanup cookies (M7)**: `expires` file = TTL 24 jam, dan GC file cookie
   `mtime > 7 hari` saat app start; dokumen perilaku ini.
8. **Regex YouTube lebih toleran** (M6): pakai pola host-based
   `(?:youtube\.com|youtu\.be|music\.youtube\.com)` lalu cek `v=` di query mana pun /
   path `/shorts/ /live/ /embed/ /v/`.
9. **Hapus `build.sh` duplikat** (K2/L3) — atau jadikan wrapper tipis ke
   `cargo build --release && bash packaging/build-deb.sh`; hapus `build.patch`;
   fix `.gitignore`; tambah `LICENSE` (MIT — README sudah mengklaim!) dan `CHANGELOG.md`;
   sinkronkan `AGENTS.md` dengan arsitektur nyata (aria2 = subprocess CLI, bukan RPC).
10. **Status info ≠ error_msg** (M10): tambah field `status_detail: String` di
    `DownloadInfo` untuk "Merging…" (badge tetap DOWNLOADING), render tanpa styling error.
11. **Tolak skema non-http(s)/ftp di `add_download`** (M11) dengan pesan error jelas.

### B. Performa & skalabilitas

1. **`tokio::process::Command` + `AsyncBufRead`** (M1): hapus `spawn_blocking`, stderr
   via `ChildStderr` di task terpisah, cek status pause/cancel **tidak per baris** tapi
   via `tokio::select!` (tick 500 ms + stream lines). Menghilangkan 2 thread/download dan
   semua `rt.block_on` di hot-path; `Handle::current()` yang rawan di blocking thread
   ikut hilang.
2. **Aria2 daemon mode (JSON-RPC)** — opsi besar yang mengubah kualitas limit:
   satu `aria2c --enable-rpc` per sesi app; keuntungan: (a) `--max-overall-download-limit`
   global sesungguhnya (fix M3), (b) pause/resume native (force pause = tulis control file)
   tanpa SIGTERM balistik, (c) koneksi & DNS reuse antar download satu host,
   (d) `aria2.getSessionInfo` menggantikan parsing regex stdout (lebih andal),
   (e) auto-retry/backoff mudah (fix L5). Bisa sebagai feature flag dengan fallback mode
   subprocess saat ini.
3. **Kurangi churn GUI**: `event_tx` unbounded → `send` per 200 ms per download sudah
   ada, tapi **stats bar** memanggil `get_all_downloads()` (kunci semua mutex) per event
   non-throttled saat status berubah — cukup kirim agregat speed dari event (running sum)
   dan hitung ulang `total` hanya saat jumlah item berubah. `promote_next` O(n) per selesai
   — untuk N≤200 tak masalah; bila session cap dinaikkan, gunakan `BTreeSet<(created, id)>`
   untuk antrean FIFO yang deterministik (fix L4 juga).
4. **Sniffer/content.js**: (a) `persist()` men-serialize ulang 50 URL JSON per event —
   batch dengan rAF/microtask; (b) `scan()` seluruh `a[href]` tiap burst 300 ms — cukup
   scan *added nodes* dari MutationObserver; (c) di situs non-YouTube, gate penuh:
   skip scan bila tidak ada elemen media setelah N detik; (d) `matches` manifest bisa
   dipersempit (mis. exclude `*://*.google.com/*` dsb.) untuk memangkas injeksi MAIN-world
   di 90% situs (host_permissions tetap `<all_urls>` untuk cookies/referer — itu memang perlu).
   (e) `chrome.storage.sync` → `local`; (f) timeout `sendToNative` + "still sending…" state.
5. **`per_process_speed_limit` adaptif**: bagi dengan **jumlah download aktif saat ini**
   (engine tahu — kirim lewat config snapshot per spawn) alih-alih `max_concurrent` statis;
   recalc saat slot berubah. (Solusi bersih tetap B2/daemon.)
6. **Resolve paralel ringan**: `resolve_client` cache per-verify; `resolve_filename`
   bisa di-*memoize* per (host, path) dengan TTL pendek untuk download massal dari
   host yang sama; `cookie_header_for` baca file sekali per host per sesi (sudah murah,
   opsional).

### C. Keamanan & privasi (hardening)

1. A + K1–K3 di atas adalah intinya; tambahan:
2. **Batas `extension_ids.json`** + konfirmasi user saat `register` (mis. notification
   "ekstensi baru terhubung") — saat ini proses lokal tanpa izin bisa mendaftarkan ID apa pun
   yang valid bentuknya ke manifest (serangan persistensi lokal).
3. **Header allow-list**: `headers` IPC diterima apa adanya ke CLI; batasi ke
   `Referer, Origin, Cookie, Authorization, Accept-Language, User-Agent` + tolak
   `Host`, `Content-Length`, `Connection` (aria2 menolak juga, tapi quick-fail lebih bersih).
4. **`--referer` eksplisit** untuk aria2 (sekarang via `--header=Referer:` — fine, tapi
   tambahkan `--http-auth-challenge` dan pertimbangkan `Netrc` bila user butuh).
5. **Redaksi URL bertoken di `session.json`/log** (opsi "redact query" untuk share-debug) —
   log `tracing::info!("Downloading: {filename}")` sudah aman, tapi `list` IPC mengembalikan
   URL penuh; tambahkan mode "ringkas".
6. Manifest extension: `clipboardRead` dipakai untuk auto-paste — pindah ke tombol
   "Paste" eksplisit (menghilangkan permission scary) atau `activeTab`-only +
   `user_gesture`; pertimbangkan pemisahan `host_permissions` per fitur dengan
   `optional_permissions` untuk cookies `<all_urls>`.

### D. Fitur (nilai jual vs IDM)

1. **Clipboard monitor** ala IDM (detect copy URL di clipboard → toast offer; via
   `wl-clipboard`/`xclip` poll 1 dtk atau `ext-global-shortcuts`) — fitur paling dirindukan.
2. **Dialog "Simpan sebagai…" per unduhan** (opsional ON/OFF) — GUI saat ini selalu
   pakai folder default; extension sudah punya `saveAs` di fallback.
3. **Proxy support** (global + per download) di config → aria2 `--all-proxy`,
   yt-dlp `--proxy` — fitur standar IDM yang belum ada.
4. **Magnet/torrent** gratis dari aria2 (`--seed-ratio`, dst.) — tinggal terima skema
   `magnet:` di resolver (flag `is_torrent`) + tampilkan `connections/seeds`.
5. **Cek integritas**: UI tambah checksum (SHA-1/MD5) → aria2 `--checksum=` (control
   file-nya sudah benar, tinggal expose).
6. **Fetch format nyata yt-dlp** (`yt-dlp -J --no-download` sekali per video) untuk
   mengisi dropdown kualitas dengan resolusi/bitrate/file-size yang sebenarnya
   (saat ini daftar statis 360p–4K; banyak video tidak punya 4K → yt-dlp diam-diam
   jatuh ke `best[height<=…]`).
7. **Jadwal & batas harian** (IDM scheduler), **rename after download** via template
   (`{title} ({res}).{ext}` — yt-dlp sudah support, tinggal expose), **search/filter list**.
8. **Tray/background mode**: saat ini tutup window = kill semua; "minimize to tray"
   (libadwaita `AdwStatusProvider` belum matang → alternatif: jangan bunuh engine saat
   window ditutup, dan andalkan notifikasi) — setidaknya opsi "tutup = hanya sembunyi".
9. **Multi-bahasa** (gettext Catalog) — UI kini hardcoded Indonesia + Inggris campur
   (`error_msg` aria2 Inggris, UI Indonesia) — konsistensikan.

### E. Kualitas kode, tes, CI

1. **CI**: tambah `cargo clippy --all-targets -- -D warnings`, `cargo deny check`
   (advisories + license), `web-ext lint` untuk extension, job `apt install`→install .deb
   smoke test nyata di LXC/container (bukan hanya `--help`), dan `cargo test` di arch
   `aarch64` (cross-compile check, karena download manager dipasang di SBC).
2. **Tes logika async engine** (selama ini tidak terjangkau unit test):
   `add_download` dedup, slot accounting (dua start paralel tak boleh lebih dari
   `max_concurrent` — regresi B3), `promote_next` FIFO, session flush/restore roundtrip —
   dengan `tempfile` + inject trait `ProcessRunner` (mock output fixture stdout aria2/
   yt-dlp nyata). Fixture stdout asli aria2 1.37/yt-dlp 2024.x disimpan di `tests/fixtures`.
3. **Refactor `spawn_supervised` + Outcome** menjadi enum state machine kecil
   (`Resolved/YtDlp/Aria2/Fallback`) agar penambahan backend (mis. http-native untuk
   situs yang butuh cookie+range manual) tidak menambah cabang if.
4. `thiserror`/`anyhow` untuk error boundary (sesuai aturan AGENTS.md sendiri; kini
   `String`-based error di seluruh engine) — minimal untuk API publik `ipc`/`engine`.
5. **Single source of version**: hapus duplikasi (L3), CI verifikasi `Cargo.toml ==
   extension/manifest.json` (ada test kecil `xtask`/shell `grep`).
6. `#[allow(dead_code)]` di `impl DownloadEngine` dan `QUALITIES` — cek ulang:
   `get_download` memang tak dipakai? Kalau API IPC bertambah (`get`), manfaatkan; kalau
   tidak, hapus.

### F. Urutan kerja yang disarankan (roadmap mini)

| Sprint | Isi |
|---|---|
| 1 (keamanan + kebenaran) | A1 socket+peercred, A2 killpg, A3 ekstensi list, A9 hapus build.sh, LICENSE, README/AGENTS fix, A4 auto-resume |
| 2 (performa) | B1 tokio::process, B3 churn GUI & sniffer, B5 limit adaptif, A5/A6/A7/M10 polish |
| 3 (fitur) | D1 clipboard, D2 save-as, D3 proxy, D6 format nyata yt-dlp |
| 4 (arsitektur) | B2 aria2 RPC mode (feature-flag), E2 integration tests engine, D8 tray |

---

## 6. Ringkasan Penilaian

Proyek ini **jauh di atas rata-rata "app pembungkus CLI"**: manajemen lock, fallback,
persistensi atomik, parsing output, dan keamanan cookie dipikirkan dengan jejak bug-fix
bernomor (B1–B20) yang didokumentasikan. Titik terlemahnya adalah **trust boundary lokal**
(socket & file di `/tmp`, satu jalur build memakai wildcard origin), **biaya thread/
pemrosesan per-baris** pada model subprocess, dan **drift dokumentasi/packaging**
(README/AGENTS/build.sh vs kode nyata). Semua dapat diperbaiki inkremental tanpa
mengubah arsitektur; yang paling berdampak per-effor: **A1+A2+A3+A9** (keamanan &
kebenaran, ±1 hari) dan **B1** (tokio::process, refactor terlokalisasi).
