//! Fast-DM binary entry point.
//!
//! Hanya berisi CLI parsing + dispatch:
//! - `fast-dm --native` → NMH mode (foreground, exit setelah 1 message)
//! - `fast-dm`          → GUI mode
//!
//! Logika di-share dengan library (`src/lib.rs`) agar integration test
//! bisa akses modul yang sama tanpa duplikasi.

use clap::Parser;
use fast_dm::{app, native_host};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "fast-dm", version, about = "Fast Download Manager")]
struct Cli {
    /// Run as Chrome native messaging host
    #[arg(long)]
    native: bool,
}

fn main() {
    // Init logging ke stderr (stdout dipakai oleh native messaging).
    // Env var RUST_LOG=fast_dm=debug untuk verbose.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("fast_dm=info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let cli = Cli::parse();

    // Mode native host = proses pendek yang di-spawn Chrome SEKALI per pesan
    // (satu unduhan dari extension = satu proses). Setup manifest browser
    // (scan profil + glob + baca/tulis manifest) TIDAK perlu diulang tiap
    // pesan: manifest awal sudah dibuat saat GUI/setup-browser.sh berjalan
    // (tanpa manifest, Chrome tidak akan bisa men-spawn native host sama
    // sekali), dan penambahan origin baru ditangani aksi `register` secara
    // langsung. Jadi jalankan setup HANYA di mode GUI — memangkas scan
    // filesystem yang redundan dari jalur panas setiap unduhan.
    if cli.native {
        // Native messaging host mode
        native_host::run();
        return;
    }

    // Auto-setup browser NMH manifests
    if let Err(e) = native_host::setup::check_and_setup() {
        tracing::warn!("NMH setup: {}", e);
    }

    // GUI mode — propagate inisialisasi error (bukan panic).
    // Kode exit 1 = init gagal (Tokio runtime, GTK build, dll).
    let app = app::FastDmApp::new();
    if let Err(e) = app.run() {
        tracing::error!("Inisialisasi gagal: {}", e);
        eprintln!("Fast DM gagal start: {}", e);
        std::process::exit(1);
    }
}
