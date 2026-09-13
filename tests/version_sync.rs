//! Sinkronisasi versi rilis — AGENTS.md §5: "sumber versi tunggal = Cargo.toml,
//! `extension/manifest.json` disinkronkan manual".
//!
//! Dua berkas itu (plus `Cargo.lock`) mudah terlupa saat bump versi, dan
//! gejalanya berbeda-beda:
//!
//! * `Cargo.lock` tertinggal → `cargo build --locked` GAGAL. CI memakai
//!   `--locked` (job `test-and-build`, langkah "Build release binary") dan
//!   `packaging/build-deb.sh` sekarang juga, jadi drift ini baru ketahuan di
//!   runner/paket, bukan di mesin dev.
//! * `extension/manifest.json` tertinggal → extension di browser tetap
//!   melaporkan versi lama. Tidak ada yang error; yang muncul adalah laporan
//!   bug "sudah update tapi perilaku lama", padahal native host dan aplikasi
//!   sudah baru.
//!
//! Test ini tidak menyentuh filesystem maupun lingkungan saat runtime: ketiga
//! berkas disematkan dengan `include_str!` (disusun oleh `concat!` +
//! `CARGO_MANIFEST_DIR`), jadi tidak ada path yang bisa meleset dan test aman
//! dijalankan paralel dengan test lain.
//!
//! Tidak butuh crate eksternal — parsing TOML/JSON di sini sengaja sesederhana
//! mungkin (KISS, AGENTS.md §3): yang dibutuhkan hanya mengambil satu nilai
//! `version` dari tiap berkas.

/// Ambil isi pasangan tanda kutip pertama pada sebuah baris (`version = "x"`
/// maupun `"version": "x"`).
fn first_quoted(line: &str) -> String {
    let start = line.find('"').expect("baris tanpa tanda kutip");
    let rest = &line[start + 1..];
    let end = rest.find('"').expect("tanda kutip tidak berpasangan");
    rest[..end].to_string()
}

/// Versi paket di bagian `[package]` pada Cargo.toml.
///
/// Pencarian dibatasi sampai header bagian berikutnya supaya `version` milik
/// dependensi (mis. `tokio = { version = "1", … }`) tidak ikut terbaca.
fn cargo_package_version(src: &str) -> String {
    let start = src
        .find("[package]")
        .expect("Cargo.toml tanpa bagian [package]");
    let body = &src[start..];
    // `body[1..]` melewati '[' milik "[package]" itu sendiri; indeks hasil
    // `find` di sana digeser +1 agar kembali ke koordinat `body`.
    let end = match body[1..].find("\n[") {
        Some(i) => i + 1,
        None => body.len(),
    };
    let section = &body[..end];
    let line = section
        .lines()
        .find(|l| l.trim_start().starts_with("version"))
        .expect("Cargo.toml [package] tanpa baris `version`");
    first_quoted(line)
}

/// Versi entri `[[package]]` bernama `package` pada Cargo.lock.
fn cargo_lock_version(src: &str, package: &str) -> String {
    let needle = format!("name = \"{package}\"");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("Cargo.lock tanpa entri `{needle}`"));
    let body = &src[start..];
    // Batasi ke blok entri ini saja (Cargo.lock memisah entri dengan baris
    // kosong) agar tidak menyambar `version` milik paket berikutnya.
    let block = match body.find("\n\n") {
        Some(i) => &body[..i],
        None => body,
    };
    let line = block
        .lines()
        .find(|l| l.trim_start().starts_with("version"))
        .unwrap_or_else(|| panic!("entri `{package}` di Cargo.lock tanpa `version`"));
    first_quoted(line)
}

/// Versi pada manifest extension (`"version": "x.y.z"`).
///
/// Kunci dicari lengkap dengan tanda kutipnya sehingga `"manifest_version"`
/// (yang muncul lebih dulu di berkas) tidak ikut cocok.
fn manifest_version(src: &str) -> String {
    let key = "\"version\"";
    let start = src
        .find(key)
        .expect("extension/manifest.json tanpa kunci \"version\"");
    let after = &src[start + key.len()..];
    let quote = after.find('"').expect("kunci \"version\" tanpa nilai");
    first_quoted(&after[quote..])
}

/// Bentuk `MAJOR.MINOR.PATCH` — tiga komponen angka, tanpa akhiran apa pun.
fn is_plain_semver(v: &str) -> bool {
    let mut parts = v.split('.');
    for _ in 0..3 {
        match parts.next() {
            Some(p) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {}
            _ => return false,
        }
    }
    parts.next().is_none()
}

#[test]
fn all_version_sources_match() {
    let cargo_toml = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let cargo_lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));
    let manifest = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/extension/manifest.json"
    ));

    let toml_version = cargo_package_version(cargo_toml);
    let lock_version = cargo_lock_version(cargo_lock, "fast-dm");
    let ext_version = manifest_version(manifest);

    assert!(
        is_plain_semver(&toml_version),
        "versi Cargo.toml `{toml_version}` bukan MAJOR.MINOR.PATCH — \
         AGENTS.md §5: bugfix +0.0.1, fitur +0.1.0"
    );

    assert_eq!(
        lock_version, toml_version,
        "Cargo.lock tertinggal dari Cargo.toml. Perbaiki dengan \
         `cargo update -p fast-dm` (atau `cargo build`) lalu commit; CI dan \
         build-deb.sh memakai --locked sehingga drift ini membuat build gagal."
    );

    assert_eq!(
        ext_version, toml_version,
        "extension/manifest.json tertinggal dari Cargo.toml. AGENTS.md §5: \
         manifest disinkronkan MANUAL saat bump versi — extension yang \
         melaporkan versi lama membuat laporan bug sulit dilacak."
    );
}

#[test]
fn version_parsers_reject_garbage() {
    // Parser di atas dibaca dari berkas nyata, jadi bentuk inputnya bisa
    // berubah tanpa disadari. Test ini mengunci perilakunya: nilai yang
    // diambil harus berasal dari bagian yang benar, bukan kebetulan cocok.
    assert_eq!(
        cargo_package_version("[package]\nversion = \"1.2.3\"\n"),
        "1.2.3"
    );

    // `version` milik dependensi di bagian lain tidak boleh ikut terbaca.
    let toml = concat!(
        "[package]\n",
        "name = \"x\"\n",
        "version = \"1.2.3\"\n",
        "\n",
        "[dependencies]\n",
        "tokio = { version = \"9\" }\n",
    );
    assert_eq!(cargo_package_version(toml), "1.2.3");

    // `version` milik entri paket berikutnya tidak boleh ikut terbaca.
    let lock = concat!(
        "[[package]]\n",
        "name = \"fast-dm\"\n",
        "version = \"1.2.3\"\n",
        "\n",
        "[[package]]\n",
        "name = \"tokio\"\n",
        "version = \"9.9.9\"\n",
    );
    assert_eq!(cargo_lock_version(lock, "fast-dm"), "1.2.3");

    // `manifest_version` muncul lebih dulu di berkas nyata — pastikan yang
    // diambil tetap `version`.
    let manifest = concat!(
        "{\n",
        "  \"manifest_version\": 3,\n",
        "  \"version\": \"1.2.3\"\n",
        "}",
    );
    assert_eq!(manifest_version(manifest), "1.2.3");
    assert!(is_plain_semver("2.11.1"));
    assert!(!is_plain_semver("2.11"));
    assert!(!is_plain_semver("2.11.1-rc1"));
    assert!(!is_plain_semver("2.11.1.0"));
    assert!(!is_plain_semver(""));
}
