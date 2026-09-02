//! Fast-DM — High-speed download manager dengan browser integration.
//!
//! Crate ini punya **dua** target:
//! - Library (`fast_dm`): module yang bisa di-test & di-import
//! - Binary (`fast-dm`): entry point CLI/GUI/NMH
//!
//! Modul publik di sini (`config`, `downloader`, `gui`, `ipc`, `native_host`)
//! dipakai oleh integration test. Modul private tetap private.
//!
//! Lihat `main.rs` untuk entry point.

pub mod app;
pub mod config;
pub mod downloader;
pub mod gui;
pub mod ipc;
pub mod native_host;

// Re-export Config karena dipakai integration test
pub use config::Config;
