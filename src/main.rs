#![allow(deprecated, dead_code, unused)]
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
    // Init logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("fast_dm=info")),
        )
        .with_target(false)
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
    } else {
        // GUI mode
        let app = app::FastDmApp::new();
        app.run();
    }
}
