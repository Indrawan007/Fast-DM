# AGENTS.md — Panduan AI/Contributor Fast-DM

Version: 0.2.0 (disinkronkan dengan arsitektur kode nyata — sebelumnya dokumen
ini menggambarkan desain lama "aria2 JSON-RPC + src/hls" yang **tidak** dipakai)

## 1. Role & Objective

- **Role:** Systems Engineer (Rust specialist) untuk Fast-DM.
- **Objective:** Merawat download manager Linux: GUI GTK4 + integrasi browser,
  dengan kecepatan didelegasikan ke **aria2c** dan **yt-dlp** sebagai subprocess CLI.

## 2. Arsitektur Nyata (baca ini sebelum menyentuh kode)

```
Browser (extension MV3: background/sniffer/content)
   │ chrome.runtime.sendNativeMessage (stdio, length-prefixed JSON)
   ▼
fast-dm --native  ──1 baris JSON──►  Unix socket (Config::ipc_socket_path(),
   │  (spawn GUI bila mati)          peer-cred uid, 0600)  ►  DownloadEngine (tokio)
                                        │ spawn_supervised: pilih backend per URL
                                        ▼
                          aria2c (subprocess, stdout di-parse regex)
                          yt-dlp (subprocess universal resolver, fallback: aria2c)
```

- **Bukan** JSON-RPC: setiap unduhan = proses `aria2c`/`yt-dlp` sendiri; kontrol
  dilakukan via sinyal (SIGTERM ke process group → resume-friendly) + parsing
  stdout (`--console-log-level`, throttle UI 5 fps).
- `src/downloader/` = `aria2.rs` (spawn+parse aria2c), `youtube.rs`
  (argumen yt-dlp + runner), `universal.rs` (resolver non-YouTube + fallback),
  `mod.rs` (engine, antrian, sesi), `types.rs` (model).
- `src/ipc/mod.rs` = server socket (download/ping/pause/resume/cancel/list/register).
- `src/native_host/` = jembatan stdio ⇄ socket + setup manifest NMH multi-browser.
- `src/gui/` = GTK4 window/dialog; state GUI disinkronkan via `mpsc::Unbounded<DownloadEvent>`.
- `extension/` = MV3: `background.js` (intercept + native msg), `sniffer.js`
  (MAIN world, hook fetch/XHR), `content.js` (overlay ⚡), `popup.*`.
- Persistensi di `~/.config/fast-dm/`: `config.json`, `session.json` (cap 200,
  flush ≤2 dtk atomik), `cookies_<host>.txt` (0600), `extension_ids.json`.
  IPC + file kerja di `XDG_RUNTIME_DIR/fast-dm` (fallback `~/.config/fast-dm/run`).

## 3. Aturan Pengembangan (ketat)

- **Minimal diff** — ubah hanya yang relevan; jangan reformat/refactor massal.
- **KISS** — hindari crate baru; yang sudah ada: tokio, gtk4, serde, reqwest,
  url, regex, dirs, clap, tracing, nix, chrono, uuid, glob, urlencoding.
- **Error idiomatik** — `Result<T, E>` (String-based ok di boundary internal);
  JANGAN `.expect()/panic` di jalur runtime user; error init dipropagasi ke main().
- **Kontrak lock-ordering** — selalu `downloads` RwLock dulu, baru `Mutex` item;
  jangan pernah terbalik (anti-deadlock, lihat `promote_next` B13).
- Semua child process di-spawn `process_group(0)`; kill hanya via
  `kill_child_pid` / `kill_child_group_hard` (SIGTERM = resume-friendly).
- Input eksternal (URL, header, nama file, cookie) WAJIB disanitasi — pola
  sudah ada di `sanitize_filename`, strip `\r\n` header, cap ukuran pesan 1 MB.
- Jangan menulis file sensitif (cookie, URL bertoken) ke path publik `/tmp`.

## 4. Tes & Verifikasi

- Unit test fungsi murni di tiap modul (`#[cfg(test)]`); integration test di
  `tests/` wajib **serial** terhadap env var global (pola `ENV_LOCK` + EnvGuard
  di `tests/find_cookies.rs`) dan tidak boleh menyentuh `~/.config` user nyata
  (override `XDG_CONFIG_HOME` ke tempdir).
- Dev-dependencies kosong dengan sengaja (stdlib-only) — jangan tambah crate
  test berat tanpa alasan kuat.
- `cargo fmt --all -- --check` adalah gerbang CI (blocking) — jalankan sebelum commit.
- `cargo clippy` advisory — ikuti bila mudah.
- Perubahan regex parser output CLI wajib menambah regression test
  (contoh: dual-format aria2 human/raw).

## 5. Versi & Rilis

- **Bug fix:** `+0.0.1` · **Fitur:** `+0.1.0` · **Breaking:** `+1.0.0`.
- Satu sumber versi: `Cargo.toml` — `extension/manifest.json` WAJIB disamakan
  manual saat rilis; GUI membaca versi via `env!("CARGO_PKG_VERSION")`.
- `EXT_ID` = extension ID stabil (dipin lewat `key` manifest) — dipakai
  `allowed_origins`; JANGAN diganti sembarangan.
- Rilis `.deb` HANYA via `packaging/build-deb.sh` (versi dibaca dari Cargo.toml).
  Tiada lagi `build.sh` (dihapus: postinst-nya memasang wildcard origin = lubang keamanan).
- `src/native_host/setup.rs` satu-satunya yang menulis manifest NMH saat runtime
  (register extension ID unpacked) — perluas daftar browser di satu tempat saja.

## 6. Keluaran Standar

- Sajikan kode lengkap, tidak terpotong; sertakan test untuk perilaku baru.
- Untuk perubahan user-facing, perbarui `README.md` + `CHANGELOG.md` di commit yang sama.
- Jangan menyimpan dependensi/artefak besar di repo; log sensitif (token URL)
  tidak boleh masuk tracing::info.
