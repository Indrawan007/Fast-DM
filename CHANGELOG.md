# Changelog

Semua perubahan notable di Fast-DM akan didokumentasikan di file ini.

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/),
dan proyek ini adheres ke [Semantic Versioning](https://semver.org/lang/id/).

## [2.2.6] — 2026-09-02

### Added
- **Unit test suite (97 tests)** untuk parser, sanitizer, dan config — sebelumnya nol test (`types.rs`, `mod.rs`, `aria2.rs`, `youtube.rs`, `config.rs`, `app.rs`)
- **Integration test** (`tests/find_cookies.rs`) — 7 test untuk `find_cookies_file` dengan tempdir manual + `XDG_CONFIG_HOME` override (tidak touch `~/.config` user, **stdlib-only — tanpa dependency eksternal**)
- **GitHub Actions CI** (`.github/workflows/ci.yml`): otomatis jalankan `cargo fmt --check`, `cargo test` (unit + integration), dan `cargo build --release` di setiap PR — 3 job: `test-and-build`, `extension`, `deb`
- **`LICENSE` file** (MIT) — sebelumnya hanya dideklarasikan di `Cargo.toml`
- **`CHANGELOG.md`** — track release history
- **CI badge** di README — status build otomatis
- **`static_regex!` macro** di `downloader/mod.rs` — panic message menjelaskan regex mana yang invalid (`name` + `pattern` + error). Menggantikan 22 `.unwrap()` polos di static regex declarations.

### Fixed
- **P0 crash fix**: `app.rs` `.expect("Failed to create tokio runtime")` diganti dengan `AppInit::try_new() → Result`. Kegagalan runtime build (mis. `ulimit -u` terlalu rendah) sekarang exit code 1 + pesan jelas, bukan panic.
- **Bug B1: `normalize_host` case-sensitive**: `trim_start_matches("www.")` di kode lama skip `"WWW."` (case-sensitive). Sekarang lowercase dulu baru strip "www." — `find_cookies_file("WWW.Example.COM")` return cookie yang benar. Ditemukan saat menulis unit test untuk `normalize_strips_www_and_lowercases`.

### Changed
- **Refactor ke lib + bin**: `src/lib.rs` baru expose modules sebagai library (`fast_dm`), `src/main.rs` jadi tipis (CLI dispatch). Memungkinkan integration test tanpa duplikasi kode.
- **CI di `.github/workflows/ci.yml`** (sebelumnya `ci/build.yml.example` yang tidak aktif).
- **`.gitignore` dirapikan**: hapus entri salah (`README.md`, `build.sh`), ganti `ekstension/` typo jadi dokumentasi di-comment.
- **Integration test pakai stdlib** (`std::env::temp_dir()` + `std::fs`), tanpa `tempfile` crate — menghindari dependency issue di sandbox dengan internet terbatas.
- **Integration test serialized via `Mutex` + `EnvGuard` RAII** — `XDG_CONFIG_HOME` adalah process-global, test paralel bisa saling overwrite env var. `EnvGuard` acquire mutex, set env, restore di `Drop` (termasuk saat panic).
- **22 `Regex::new(...).unwrap()` diganti dengan `static_regex!(name, pattern)`** — kalau ada typo di pola regex, panic message sekarang jelas menunjukkan regex mana yang invalid (mis. `regex "aria2_eta" invalid (pattern="ETA:(\\S+)"): ...`).

## [2.2.5] — dan sebelumnya

Lihat git history untuk perubahan pre-P0.
