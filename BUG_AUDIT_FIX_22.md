# Audit Bug 22 — Fast-DM v3.2.9 → Fix

Format: K = Kritis, M = Medium, L = Low. Semua diperbaiki di branch `arena/01a0d06a-fast-dm`.

## Kritis (K1-K11)

- **K1 clipboard probe cache None** — `gui/window.rs`: `cached_tool` sebelumnya `Some(None)` membuat loop tidak pernah probe lagi setelah satu kegagalan. Fix: `cached_tool = None` dan re-probe tiap tick (sudah ada) + M3 tambahan.
- **K2 TOCTOU port** — `aria2_rpc.rs::ensure_daemon`: `TcpListener::bind` lalu drop = race port direbut proses lain. Fix: `TcpStream::connect` probe 200ms untuk deteksi port in-use tanpa bind.
- **K3 rpc_secret_in invalid file** — `config.rs::rpc_secret_in`: file ada tapi kosong/kepanjangan → tiap start fresh berbeda sementara file invalid tetap. Fix: hapus file invalid sebelum generate fresh.
- **K4 freshness cookie** — `aria2.rs`, `youtube.rs`, `aria2_rpc.rs`: `find_cookies_file` hanya cek exist, cookie basi tetap memaksa fallback ke per-proses. Fix: `COOKIE_FRESH_SECS = 24h`, `is_fresh_cookie_file` + `find_fresh_cookies_file` dipakai di semua jalur (aria2 build_aria2_cmd, cookie_header_for, youtube cookie_args, aria2_rpc has_cookie_jar).
- **K5 config corrupt backup** — `config.rs::load_startup_snapshot`: config rusak langsung pakai default tanpa backup. Fix: backup ke `config.corrupt-<millis>-<rand>.json` via copy/rename best-effort.
- **K6 save_dir validation** — `config.rs`: folder unduhan tidak divalidasi, path aneh (`..`, relatif, control char) bikin aria2 gagal tanpa pesan jelas. Fix: `is_valid_download_dir` (absolute, len≤4096, no control, no ParentDir, reject "/" & "/tmp") dipakai di `update_config`, `add_download` fallback, dan GUI inline validation dengan error label.
- **K7 unique_filename timestamp collision** — `aria2.rs::unique_filename`: fallback `timestamp_millis()` saja bisa tabrakan di ms sama. Fix: tambah 6 char random uuid.
- **K8 sanitize_filename fallback collision** — `downloader/mod.rs::sanitize_filename`: fallback `download_<timestamp detik>` rawan tabrakan detik sama. Fix: millis + 4 char random.
- **K9 extract_filename_from_url fallback collision** — sama seperti K8, fallback detik. Fix: millis + random.
- **K10 config load validation** — `config.rs::parse_config`: field hasil edit manual invalid (download_dir relatif, max_connections 0, rpc_port 0, proxy ngawur, speed invalid) lolos dan bikin aria2 gagal start. Fix: validasi di level load dengan fallback ke default + warn.
- **K11 rpc_port 0** — `mod.rs::update_config` tidak cek rpc_port 0. Fix: tolak 0 dengan pesan "RPC port harus >0".

## Medium (M1-M8)

- **M1 speed limit spasi internal** — `is_valid_speed_limit`: "512 K" lolos padahal aria2 tolak. Fix: tolak whitespace di dalam string.
- **M2 proxy inline validation GUI** — `gui/window.rs`: proxy invalid hanya muncul sebagai label tombol "Gagal Simpan", user tidak tahu field mana. Fix: tambah `proxy_box` + `proxy_error` label, validasi inline seperti speed & folder.
- **M3 clipboard re-probe on failure** — `gui/window.rs`: `cached_tool` Some tapi `clipboard_text` None (tool hilang) → loop keep failing tool tanpa re-probe. Fix: kosongkan cache bila text None.
- **M4 shutdown_daemon stale GID** — `aria2_rpc.rs::shutdown_daemon`: bila daemon tidak merespons dan gids tidak kosong, return Err sehingga GID dipertahankan di session.json (basi). Fix: selalu Ok bila tidak reachable, GID dibersihkan, next start langsung addUri (resume via .aria2 file tetap jalan).
- **M5 create_dir_all silent** — `mod.rs::add_download`: `create_dir_all` ignore error → download gagal dengan pesan membingungkan. Fix: log warn bila gagal.
- **M6 root/tmp rejection** — `is_valid_download_dir`: "/" dan "/tmp" diterima, berbahaya. Fix: tolak eksplisit.
- **M7 youtube cookie host normalization** — `youtube.rs::cookie_args`: host mentah "WWW.Example.COM" gagal lookup padahal file ada. Fix: lowercase + strip www. sebelum lookup, fallback ke host asli.
- **M8 config backup collision** — backup corrupt pakai detik saja bisa tabrakan. Fix: millis + random (sudah di K5).

## Low (L1-L5)

- **L1 cookies_file_in_host length** — nama host sangat panjang bisa melebihi batas filename 255. Fix: truncate 200 char.
- **L2 session.json backup collision** — backup session corrupt pakai detik. Fix: millis + random.
- **L3 speed validation duplikat di config** — `config.rs` butuh validasi speed tanpa circular dep. Fix: `is_valid_speed_limit_cfg` dengan whitespace check.
- **L4 proxy length limit** — `is_valid_proxy_url` tanpa batas panjang → DoS argumen CLI. Fix: len>2048 tolak.
- **L5 config backup with_extension dot handling** — `with_extension` dengan string mengandung dot tetap valid, tapi pakai format jelas `corrupt-<millis>-<rand>.json`.

## Verifikasi

- Cargo tidak tersedia di sandbox, verifikasi manual via `git diff` dan review logika.
- Semua jalur cookie kini konsisten pakai `find_fresh_cookies_file`.
- GUI settings: folder, speed, proxy semua punya error label inline + hide on changed.
- Fallback filename & backup file semua pakai millis + random anti-tabrakan.

## File diubah

- src/config.rs
- src/downloader/aria2.rs
- src/downloader/aria2_rpc.rs
- src/downloader/mod.rs
- src/downloader/youtube.rs
- src/gui/window.rs
