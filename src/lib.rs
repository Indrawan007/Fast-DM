//! Fast-DM — download manager Arch Linux untuk Hyprland/Wayland dengan
//! integrasi browser.
//!
//! Crate ini punya **dua** target:
//! - Library (`fast_dm`): module yang bisa di-test & di-import
//! - Binary (`fast-dm`): entry point CLI/GUI/NMH
//!
//! Modul publik di sini (`config`, `downloader`, `gui`, `ipc`, `native_host`,
//! `pkg`) dipakai oleh integration test. `pkg` hanya menyediakan petunjuk
//! pacman untuk target Arch Linux. Modul private tetap private.
//!
//! Lihat `main.rs` untuk entry point.

pub mod app;
pub mod config;
pub mod downloader;
pub mod gui;
pub mod ipc;
pub mod native_host;
pub mod pkg;

// Re-export Config karena dipakai integration test
pub use config::Config;
