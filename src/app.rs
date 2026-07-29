use crate::downloader::{DownloadEngine, types::DownloadEvent};
use crate::gui;
use crate::ipc;

use gtk4::prelude::*;
use gtk4::Application;
use tokio::sync::mpsc;
use std::sync::Arc;

const APP_ID: &str = "com.fastdm.app";

pub struct FastDmApp;

impl FastDmApp {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&self) {
        let app = Application::builder()
            .application_id(APP_ID)
            .build();

        app.connect_activate(move |app| {
            let (event_tx, event_rx) = mpsc::unbounded_channel::<DownloadEvent>();

            // Buat tokio runtime yang TIDAK di-drop
            let rt = Box::leak(Box::new(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime")
            ));

            let rt_handle = rt.handle().clone();

            let engine = Arc::new(
                rt.block_on(async { DownloadEngine::new(event_tx) })
            );

            // IPC server di background thread
            let engine_ipc = engine.clone();
            let rt_handle_ipc = rt_handle.clone();
            std::thread::Builder::new()
                .name("ipc-server".into())
                .spawn(move || {
                    rt_handle_ipc.block_on(async {
                        if let Err(e) = ipc::start_server(engine_ipc).await {
                            tracing::error!("IPC server error: {}", e);
                        }
                    });
                })
                .ok();

            gui::window::build_window(app, engine.clone(), event_rx, rt_handle);
        });

        app.run();
    }
}
