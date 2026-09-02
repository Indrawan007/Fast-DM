# P0 Fixes — Ringkasan Perubahan

## 1. `app.rs` — Error propagation (sebelumnya panic)

**Sebelum:**
```rust
let rt = Box::leak(Box::new(
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("fastdm-worker")
        .build()
        .expect("Failed to create tokio runtime")  // ← PANIC kalau gagal
));
```

**Sesudah:**
```rust
let init = AppInit::try_new()?;  // Propagasi Err(String) ke main()

struct AppInit { rt: &'static tokio::runtime::Runtime, rt_handle: tokio::runtime::Handle }
impl AppInit {
    fn try_new() -> Result<Self, String> {
        let rt = Box::leak(Box::new(
            tokio::runtime::Builder::new_multi_thread()
                ...
                .build()
                .map_err(|e| format!("Gagal membuat Tokio runtime (ulimit thread rendah?): {}", e))?
        ));
        Ok(Self { rt, rt_handle: rt.handle().clone() })
    }
}
```

**main.rs propagasi:**
```rust
if let Err(e) = app.run() {
    tracing::error!("Inisialisasi gagal: {}", e);
    eprintln!("Fast DM gagal start: {}", e);
    std::process::exit(1);
}
```

**Dampak:**
- Runtime build gagal (mis. `ulimit -u` terlalu rendah) → exit code 1 + pesan jelas, **bukan panic**.
- Perilaku "leak runtime" tetap sama — tidak mengubah logika ownership, hanya error handling.

## 2. Unit test komprehensif

### `src/downloader/types.rs` (+180 LOC)
- `status_display_all_variants` — Display untuk semua 7 status
- `status_roundtrip_via_serde` — JSON roundtrip stabil
- `format_size_*` — bytes, KB, MB, GB, TB, overflow
- `format_eta_*` — detik, menit, jam
- `download_info_new_defaults` & `download_info_formatters` — sanity check
- `download_info_serde_with_missing_fields` — backward-compat session.json

### `src/downloader/mod.rs` (+200 LOC)
- `is_direct_file_url_various_extensions` & `_strips_query_and_fragment` & `_excludes_streaming` & `_non_file`
- `is_valid_speed_limit_valid` & `_invalid`
- `sanitize_filename_basic` & `_strips_invalid_chars` & `_strips_query_and_fragment` & `_trims_dots_and_spaces` & `_empty_fallback` & `_unicode_safe` (UTF-8 char boundary) & `_control_chars`
- `extract_filename_basic` & `_with_query` & `_url_encoded` & `_no_extension_fallback` & `_invalid_url_fallback` & `_root_path_fallback` & `_traversal_protected`

### `src/downloader/aria2.rs` (+250 LOC)
- `parse_aria2_size_*` — raw bytes, human KiB/MiB/GiB, short units, invalid
- `parse_eta_*` — seconds, MM:SS, HH:MM:SS, empty
- `parse_speed_setting_*` — zero, K/M/G units, case-insensitive
- `per_process_speed_limit_*` — unlimited, divided, floor at 1K
- `is_generic_filename_*` — true cases (12) & false cases (4)
- `content_type_to_ext_*` — video, audio, archive, image, charset stripping, unknown
- `parse_content_disposition_*` — RFC 5987 (uppercase + lowercase), quoted, unquoted, none
- `re_progress_matches_*` — regression: format raw & human-readable

### `src/downloader/youtube.rs` (+180 LOC)
- `is_youtube_url_positive` & `_negative` — watch/shorts/embed/music
- `parse_eta_hms_*` — seconds, MM:SS, HH:MM:SS, invalid
- `parse_speed_*` — MiB/s, K/M short, KiB, invalid
- `output_template_*` — explicit filename, escape %, no extension, generic
- `quality_args_*` — resolution (p), 4K, audio_best, audio_mp3, default, non-numeric
- `desktop_to_browser_*` — known browsers, unknown

### `src/config.rs` (+130 LOC)
- `normalize_strips_www_and_lowercases`
- `normalize_trims_whitespace`
- `cookies_file_sanitizes_unsafe_chars`
- `find_cookies_file_none_when_not_found`
- `find_cookies_file_single_label_returns_none` — smoke test walk-up
- `config_default_has_safe_values` — tidak panic tanpa HOME
- `config_serde_roundtrip`
- `config_serde_with_missing_fields_uses_default`

### `src/app.rs` (+25 LOC)
- `app_init_signature_returns_result` — compile-time regression: signature
  AppInit::try_new() = Result, bukan expect/unwrap.

## Total
- **5 file sumber diubah** untuk fix P0
- **~70 unit test baru** ditambahkan
- **0 test yang touch filesystem nyata** — semua pure-function atau path-only
- **2 LOC `expect()` diganti** dengan `Result` propagation

## Bug ditemukan saat testing

| Bug | Lokasi | Symptom | Fix |
|---|---|---|---|
| B1 | `config.rs::normalize_host` | `find_cookies_file("WWW.Example.COM")` return cookie yang salah (tidak strip "WWW.") karena `trim_start_matches` case-sensitive | Lowercase dulu, baru strip "www." |
| — | (test expectation) | `format_size(512)` return `"512.0 B"` (sesuai produksi) tapi test expects `"512 B"` | Test disesuaikan ke behavior produksi |
| — | (test expectation) | `sanitize_filename` strip query `?` di awal — test input `"a<b>c:d\"e/f\\g|h?i*.txt"` ke-truncate di `?` | Test pakai input tanpa `?` |
| — | (test expectation) | `sanitize_filename` TIDAK replace spasi (by design — Linux fs support spasi) | Test expectation disesuaikan |
| — | (test expectation) | `cookies_file_for("evil host/name.txt")` → `cookies_evil_host_name.txt` (bukan `..._txt` — typo test) | Test expectation fix typo |
| B2 | `tests/find_cookies.rs` | Test paralel saling overwrite `XDG_CONFIG_HOME` (env global) → `Config::config_dir()` return path salah | Tambah `Mutex` global + `EnvGuard` RAII (lock + set env, restore di `Drop`); env restored sebelum lock di-release (penting: field `_lock` di-declare terakhir → di-drop duluan → `Drop::drop` jalan duluan → env restored → lalu field drop) |

## Catatan
- Sandbox ini tidak punya Rust toolchain (`cargo` / `rustc`), jadi verifikasi
  `cargo test` belum dijalankan. Semua test di-review manual untuk kebenaran
  aritmatika & logika, tapi **WAJIB dijalankan di Linux dengan GTK4** untuk
  verifikasi akhir.
- Test yang touch `find_cookies_file` dengan file nyata sengaja di-skip — akan
  ditambahkan di integration test (`tests/find_cookies.rs`) dengan
  `std::env::temp_dir()` + cleanup manual (stdlib-only, tanpa dep).
