mod app;
mod config;
mod downloader;
mod gui;
mod ipc;
mod native_host;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "fast-dm", version, about = "Fast Download Manager")]
struct Cli {
    /// Run as Chrome native messaging host
    #[arg(long)]
    native: bool,
}

fn main() {
    // Init logging (stderr agar tidak mengganggu native messaging stdout)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("fast_dm=info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let cli = Cli::parse();

    // Auto-setup browser NMH manifests
    if let Err(e) = native_host::setup::check_and_setup() {
        tracing::warn!("NMH setup: {}", e);
    }

    if cli.native {
        // Native messaging host mode
        native_host::run();
        return;
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
