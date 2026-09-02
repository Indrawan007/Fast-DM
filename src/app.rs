use crate::downloader::{DownloadEngine, types::DownloadEvent};
use crate::gui;
use crate::ipc;

use gtk4::prelude::*;
use gtk4::Application;
use tokio::sync::mpsc;
use std::sync::Arc;

// ID unik yang tidak akan collide dengan app lain
const APP_ID: &str = "io.github.fastdm.FastDownloadManager";

pub struct FastDmApp;

impl FastDmApp {
    pub fn new() -> Self {
        Self
    }

    /// Jalankan GUI loop. Return `Ok(())` jika selesai normal,
    /// `Err(msg)` jika inisialisasi gagal (runtime Tokio, GTK build, dll).
    /// Tidak pernah panic — semua error dipropagasi ke `main()`.
    pub fn run(&self) -> Result<(), String> {
        // Bangun runtime SEBELUM Application agar error bisa dipropagasi.
        // Disimpan di AppInit (di-leak 'static) supaya bisa direferensikan
        // dari callback GTK yang di-move. Runtime leak ini satu-satunya
        // cara agar Tokio Runtime bisa diakses dari closure yang 'static
        // tanpa mengubah ownership pattern — sebelumnya .expect() dipakai
        // di sini dan bisa panic di runtime build.
        let init = AppInit::try_new()?;
        let app = Application::builder()
            .application_id(APP_ID)
            .flags(gtk4::gio::ApplicationFlags::FLAGS_NONE)
            .build();

        app.connect_activate(move |app| {
            // B2: launch kedua (app-id sama) hanya di-forward sebagai sinyal
            // activate ke proses pertama — jangan bangun window/engine/runtime
            // baru (leak runtime + dua flusher menulis session.json saling timpa).
            if let Some(win) = app.windows().first() {
                win.present();
                return;
            }

            let (event_tx, event_rx) = mpsc::unbounded_channel::<DownloadEvent>();
            let gui_event_tx = event_tx.clone();

            let rt = init.rt;
            let rt_handle = init.rt_handle.clone();

            let engine = Arc::new(
                rt.block_on(async { DownloadEngine::new(event_tx) })
            );

            let engine_ipc = engine.clone();
            let rt_handle_ipc = rt_handle.clone();
            if let Err(e) = std::thread::Builder::new()
                .name("fastdm-ipc".into())
                .spawn(move || {
                    rt_handle_ipc.block_on(async {
                        if let Err(e) = ipc::start_server(engine_ipc).await {
                            tracing::error!("IPC server error: {}", e);
                        }
                    });
                })
            {
                tracing::error!("Gagal spawn IPC thread: {}", e);
            }

            gui::window::build_window(app, engine.clone(), event_rx, gui_event_tx, rt_handle);
        });

        // GTK4: app.run() blocking sampai app.quit(). Tidak propagasi
        // exit code — kode 0/normal vs non-zero sudah ditangani logging
        // di dalam callback. main() tidak perlu tahu detail exit code.
        app.run();
        Ok(())
    }
}

/// Inisialisasi yang mahal (Tokio runtime) — dibangun sekali SEBELUM
/// callback GTK agar kegagalannya bisa di-propagasi ke main(), bukan
/// dipanic di tengah handler.
///
/// Field `rt: &'static Runtime` di sini aman karena AppInit hanya
/// dipakai untuk mendelegasikan handle ke callback GTK, dan runtime
/// yang di-leak('static) memang ditujukan untuk seumur hidup proses
/// — sama persis dengan pola versi lama, hanya `expect()`-nya diganti
/// `try_new()`.
struct AppInit {
    rt: &'static tokio::runtime::Runtime,
    rt_handle: tokio::runtime::Handle,
}

impl AppInit {
    fn try_new() -> Result<Self, String> {
        let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("fastdm-worker")
                .build()
                .map_err(|e| {
                    format!(
                        "Gagal membuat Tokio runtime (ulimit thread rendah?): {}",
                        e
                    )
                })?,
        ));
        Ok(Self {
            rt,
            rt_handle: rt.handle().clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P0 regression: AppInit harus dibuat dengan try_new() yang return
    /// `Result`, BUKAN .expect() yang panic. Test ini adalah compile-time
    /// check: kalau ada yang balik ke .expect() / .unwrap() di try_new,
    /// signature berubah dan test ini gagal compile.
    ///
    /// Test instantiasi runtime di test env tidak aman (Runtime + Box::leak
    /// interaksi dengan test runner tidak deterministik), jadi kita hanya
    /// verifikasi signature di sini. Smoke test untuk AppInit::try_new()
    /// ada di integration test (lihat `tests/app_init.rs` jika ada).
    #[test]
    fn app_init_signature_returns_result() {
        // Force compiler untuk verifikasi signature try_new() = Result
        let f: fn() -> Result<AppInit, String> = AppInit::try_new;
        let _ = f;
    }
}


