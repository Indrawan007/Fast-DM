use crate::config::Config;
use crate::downloader::types::*;
use crate::downloader::DownloadEngine;
use crate::gui::css;
use crate::gui::download_row::DownloadRow;
use crate::gui::youtube_dialog;

use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider,
    Entry, Label, ListBox, Orientation, PolicyType, ScrolledWindow,
    SelectionMode,
};
use glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc;

pub fn build_window(
    app: &Application,
    engine: Arc<DownloadEngine>,
    mut event_rx: mpsc::UnboundedReceiver<DownloadEvent>,
    gui_tx: mpsc::UnboundedSender<DownloadEvent>,
    rt: tokio::runtime::Handle,
) {

    // Load CSS — scoped ke window saja
    let provider = CssProvider::new();
    provider.load_from_string(css::THEME_CSS);

    // Window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Fast Download Manager")
        .default_width(780)
        .default_height(580)
        .build();

    // GTK4 tidak punya set_icon() di Window
    // Icon di-set via .desktop file field "Icon=io.github.fastdm.FastDownloadManager"
    // dan file icon di /usr/share/icons/hicolor/*/apps/

    let root = GtkBox::new(Orientation::Vertical, 0);

    // ── Header ──
    let header = GtkBox::new(Orientation::Horizontal, 10);
    header.add_css_class("header-box");

    let title = Label::new(Some("\u{26A1} Fast Download Manager"));
    title.add_css_class("header-title");

    let subtitle = Label::new(Some("RUST + ARIA2"));
    subtitle.add_css_class("header-subtitle");
    subtitle.set_valign(gtk4::Align::End);

    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    // ── Toolbar ──
    let toolbar = GtkBox::new(Orientation::Horizontal, 8);
    toolbar.add_css_class("toolbar-box");

    let url_entry = Entry::new();
    url_entry.set_placeholder_text(Some("Paste download URL here..."));
    url_entry.set_hexpand(true);
    url_entry.add_css_class("url-entry");

    let add_btn = Button::with_label("Download");
    add_btn.add_css_class("btn-download");

    let clear_btn = Button::with_label("Clear Done");
    clear_btn.add_css_class("btn-clear");

    let settings_btn = Button::with_label("Settings");
    settings_btn.add_css_class("btn-clear");

    toolbar.append(&url_entry);
    toolbar.append(&add_btn);
    toolbar.append(&clear_btn);
    toolbar.append(&settings_btn);
    root.append(&toolbar);

    // ── List ──
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let listbox = ListBox::new();
    listbox.set_selection_mode(SelectionMode::None);
    listbox.add_css_class("download-list");

    // Placeholder
    let placeholder = GtkBox::new(Orientation::Vertical, 8);
    placeholder.set_valign(gtk4::Align::Center);
    placeholder.set_margin_top(60);
    placeholder.set_margin_bottom(60);

    let ph_icon = Label::new(Some("\u{26A1}"));
    ph_icon.add_css_class("ph-icon");
    let ph_title = Label::new(Some("No downloads yet"));
    ph_title.add_css_class("ph-title");
    let ph_sub = Label::new(Some("Paste a URL above or use the browser extension"));
    ph_sub.add_css_class("ph-sub");

    placeholder.append(&ph_icon);
    placeholder.append(&ph_title);
    placeholder.append(&ph_sub);
    listbox.set_placeholder(Some(&placeholder));

    scroll.set_child(Some(&listbox));
    root.append(&scroll);

    // ── Stats ──
    let statsbar = GtkBox::new(Orientation::Horizontal, 24);
    statsbar.add_css_class("stats-box");

    let stats_active = Label::new(Some("Active 0"));
    stats_active.add_css_class("stats-value");
    let stats_speed = Label::new(Some("0 B/s"));
    stats_speed.add_css_class("stats-speed");
    let stats_total = Label::new(Some("Total 0"));
    stats_total.add_css_class("stats-value");

    statsbar.append(&stats_active);
    statsbar.append(&stats_speed);
    statsbar.append(&stats_total);
    root.append(&statsbar);

    root.add_css_class("fast-dm-app");
    window.add_css_class("fast-dm-window");

    let display = gtk4::prelude::RootExt::display(&window);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    window.set_child(Some(&root));

    // ── State ──
    let rows: Rc<RefCell<HashMap<String, DownloadRow>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // Track download status for clear done
    let download_statuses: Rc<RefCell<HashMap<String, DownloadStatus>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // ── Add URL handler ──
    let engine_add = engine.clone();
    let rt_add = rt.clone();
    let entry_add = url_entry.clone();

    let win_add = window.clone();

    let on_add = move || {
        let url = entry_add.text().to_string().trim().to_string();
        if url.is_empty() {
            return;
        }

        let url = if !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("ftp://")
        {
            format!("https://{}", url)
        } else {
            url
        };

        entry_add.set_text("");

        // YouTube: minta pilihan kualitas langsung dari GUI (dialog yang tadinya dead-code)
        let quality = if crate::downloader::youtube::is_youtube_url(&url) {
            youtube_dialog::show_quality_dialog(
                win_add.upcast_ref(),
                "Pilih kualitas video",
                "",
                "",
            )
        } else {
            None
        };

        let eng = engine_add.clone();
        let rt = rt_add.clone();

        glib::spawn_future_local(async move {
            let _ = rt.spawn(async move {
                eng.add_download(&url, None, None, true, Default::default(), quality).await
            }).await;
        });
    };

    let on_add_clone = on_add.clone();
    add_btn.connect_clicked(move |_| on_add_clone());

    let on_add_clone = on_add.clone();
    url_entry.connect_activate(move |_| on_add_clone());

    // ── Clear Done handler ──
    let rows_clear = rows.clone();
    let listbox_clear = listbox.clone();
    let engine_clear = engine.clone();
    let rt_clear = rt.clone();
    let statuses_clear = download_statuses.clone();

    clear_btn.connect_clicked(move |_| {
        let statuses = statuses_clear.borrow();
        let to_remove: Vec<String> = statuses.iter()
            .filter(|(_, status)| {
                matches!(status,
                    DownloadStatus::Completed |
                    DownloadStatus::Cancelled
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        drop(statuses);

        for id in &to_remove {
            // Remove row from listbox
            let mut rows_map = rows_clear.borrow_mut();
            if let Some(row) = rows_map.remove(id) {
                listbox_clear.remove(&row.root);
            }
            drop(rows_map);

            // Clear from engine
            let eng = engine_clear.clone();
            let id_for_engine = id.clone();
            let rt = rt_clear.clone();
            glib::spawn_future_local(async move {
                let _ = rt.spawn(async move {
                    eng.clear_download(&id_for_engine).await;
                }).await;
            });
        }

        // Remove from status tracking (setelah loop selesai)
        let mut st = statuses_clear.borrow_mut();
        for id in &to_remove {
            st.remove(id);
        }
    });

    // ── Settings handler ──
    let win_settings = window.clone();
    let engine_settings = engine.clone();
    let rt_settings = rt.clone();
    let settings_btn_h = settings_btn.clone();
    settings_btn.connect_clicked(move |_| {
        // Baca config saat ini (main thread → block_on aman)
        let eng = engine_settings.clone();
        let cur = rt_settings.block_on(async move { eng.get_config().await });

        let Some(cfg) = show_settings_dialog(win_settings.upcast_ref(), &cur) else {
            return;
        };

        // Simpan ke disk + apply live; tampilkan feedback (termasuk kalau validasi gagal)
        let eng2 = engine_settings.clone();
        let handle = rt_settings.spawn(async move {
            eng2.update_config(cfg).await
        });
        let btn_feedback = settings_btn_h.clone();
        glib::spawn_future_local(async move {
            match handle.await {
                Ok(Ok(())) => btn_feedback.set_label("✓ Saved"),
                Ok(Err(e)) => {
                    tracing::warn!("Settings rejected: {}", e);
                    btn_feedback.set_label("✕ Invalid");
                }
                Err(_) => btn_feedback.set_label("✕ Failed"),
            }
            let btn = btn_feedback.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                btn.set_label("Settings");
                glib::ControlFlow::Break
            });
        });
    });

    // ── Event listener ──
    let app_ev = app.clone();
    let rows_ev = rows.clone();
    let listbox_ev = listbox.clone();
    let engine_ev = engine.clone();
    let rt_ev = rt.clone();
    let stats_a = stats_active.clone();
    let stats_s = stats_speed.clone();
    let stats_t = stats_total.clone();
    let statuses_ev = download_statuses.clone();

    glib::spawn_future_local(async move {
        let mut last_stats = std::time::Instant::now();
        while let Some(event) = event_rx.recv().await {
            let info = match &event {
                DownloadEvent::Progress(i) => i,
                DownloadEvent::Completed(i) => i,
                DownloadEvent::Error(i) => i,
            };

            // Track status
            statuses_ev.borrow_mut().insert(
                info.id.clone(), info.status
            );

            // Notifikasi desktop saat selesai / gagal
            match &event {
                DownloadEvent::Completed(i) => {
                    let n = gtk4::gio::Notification::new("Download selesai");
                    n.set_body(Some(&i.filename));
                    app_ev.send_notification(Some(&i.id), &n);
                }
                DownloadEvent::Error(i) => {
                    let n = gtk4::gio::Notification::new("Download gagal");
                    n.set_body(Some(&i.filename));
                    app_ev.send_notification(Some(&i.id), &n);
                }
                _ => {}
            }

            let mut rows_map = rows_ev.borrow_mut();

            if let Some(row) = rows_map.get_mut(&info.id) {
                row.update(info);
            } else {
                // New download — create row
                let row = DownloadRow::new(info);

                // Connect buttons
                let id = info.id.clone();
                let eng = engine_ev.clone();
                let rt = rt_ev.clone();

                // Pause
                let id_p = id.clone();
                let eng_p = eng.clone();
                let rt_p = rt.clone();
                row.pause_btn.connect_clicked(move |_| {
                    let eng = eng_p.clone();
                    let id = id_p.clone();
                    let rt = rt_p.clone();
                    glib::spawn_future_local(async move {
                        let _ = rt.spawn(async move {
                            eng.pause_download(&id).await
                        }).await;
                    });
                });

                // Resume
                let id_r = id.clone();
                let eng_r = eng.clone();
                let rt_r = rt.clone();
                row.resume_btn.connect_clicked(move |_| {
                    let eng = eng_r.clone();
                    let id = id_r.clone();
                    let rt = rt_r.clone();
                    glib::spawn_future_local(async move {
                        let _ = rt.spawn(async move {
                            eng.resume_download(&id).await
                        }).await;
                    });
                });

                // Cancel
                let id_c = id.clone();
                let eng_c = eng.clone();
                let rt_c = rt.clone();
                row.cancel_btn.connect_clicked(move |_| {
                    let eng = eng_c.clone();
                    let id = id_c.clone();
                    let rt = rt_c.clone();
                    glib::spawn_future_local(async move {
                        let _ = rt.spawn(async move {
                            eng.cancel_download(&id).await
                        }).await;
                    });
                });

                // Retry
                let id_t = id.clone();
                let eng_t = eng.clone();
                let rt_t = rt.clone();
                row.retry_btn.connect_clicked(move |_| {
                    let eng = eng_t.clone();
                    let id = id_t.clone();
                    let rt = rt_t.clone();
                    glib::spawn_future_local(async move {
                        let _ = rt.spawn(async move {
                            eng.resume_download(&id).await
                        }).await;
                    });
                });

                // Open folder
                let save_dir = info.save_dir.clone();
                row.open_btn.connect_clicked(move |_| {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&save_dir)
                        .spawn();
                });

                // Remove — hapus dari daftar, file TETAP ada
                let id_rm = id.clone();
                let eng_rm = eng.clone();
                let rt_rm = rt.clone();
                let rows_rm = rows_ev.clone();
                let listbox_rm = listbox_ev.clone();
                let statuses_rm = statuses_ev.clone();
                row.remove_btn.connect_clicked(move |_| {
                    // Remove row
                    let mut rmap = rows_rm.borrow_mut();
                    if let Some(r) = rmap.remove(&id_rm) {
                        listbox_rm.remove(&r.root);
                    }
                    drop(rmap);

                    // Clear from engine
                    let eng = eng_rm.clone();
                    let id = id_rm.clone();
                    let rt = rt_rm.clone();
                    glib::spawn_future_local(async move {
                        let _ = rt.spawn(async move {
                            eng.clear_download(&id).await;
                        }).await;
                    });

                    // Remove status
                    statuses_rm.borrow_mut().remove(&id_rm);
                });

                listbox_ev.prepend(&row.root);
                rows_map.insert(id, row);
            }

            // Update stats — selalu refresh saat status berubah;
            // throttle 500ms saat download aktif (hindari spawn task 5x/detik per download)
            drop(rows_map);
            let is_active = matches!(
                info.status,
                DownloadStatus::Downloading | DownloadStatus::Resolving
            );

            if !is_active || last_stats.elapsed().as_millis() >= 500 {
                last_stats = std::time::Instant::now();

                let eng = engine_ev.clone();
                let rt = rt_ev.clone();
                let sa = stats_a.clone();
                let ss = stats_s.clone();
                let st = stats_t.clone();

                glib::spawn_future_local(async move {
                    if let Ok(all) = rt.spawn(async move {
                        eng.get_all_downloads().await
                    }).await {
                        let active: Vec<_> = all.iter()
                            .filter(|d| matches!(d.status, DownloadStatus::Downloading))
                            .collect();
                        let total_speed: u64 = active.iter().map(|d| d.speed).sum();

                        sa.set_text(&format!("Active {}", active.len()));
                        ss.set_text(&if total_speed > 0 {
                            format!("{}/s", format_size(total_speed))
                        } else {
                            "0 B/s".to_string()
                        });
                        st.set_text(&format!("Total {}", all.len()));
                    }
                });
            }
        }
    });

    // Seed rows dari session hasil restore (persistensi antar restart)
    {
        let engine_seed = engine.clone();
        let rt_seed = rt.clone();
        glib::spawn_future_local(async move {
            if let Ok(all) = rt_seed
                .spawn(async move { engine_seed.get_all_downloads().await })
                .await
            {
                for d in all {
                    let _ = gui_tx.send(DownloadEvent::Progress(d));
                }
            }
        });
    }

    // Kill semua child process (aria2c/yt-dlp) saat window ditutup — anti orphan
    let engine_close = engine.clone();
    let rt_close = rt.clone();
    window.connect_close_request(move |_| {
        let eng = engine_close.clone();
        let all = rt_close.block_on(async move { eng.get_all_downloads().await });
        for d in all {
            if let Some(pid) = d.pid {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
        glib::Propagation::Proceed
    });

    window.present();
}

fn settings_row(label: &str, widget: &impl IsA<gtk4::Widget>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    let lbl = Label::new(Some(label));
    lbl.set_hexpand(true);
    lbl.set_halign(gtk4::Align::Start);
    row.append(&lbl);
    row.append(widget);
    row
}

/// Dialog settings — perubahan berlaku untuk download baru tanpa restart
#[allow(deprecated)] // gtk4::Dialog deprecated sejak 4.10 — pola sama seperti youtube_dialog.rs
fn show_settings_dialog(parent: &gtk4::Window, cur: &Config) -> Option<Config> {
    let dialog = gtk4::Dialog::with_buttons(
        Some("Settings"),
        Some(parent),
        gtk4::DialogFlags::MODAL | gtk4::DialogFlags::DESTROY_WITH_PARENT,
        &[],
    );
    dialog.set_default_size(420, -1);

    let content = dialog.content_area();
    content.set_spacing(10);
    content.set_margin_top(16);
    content.set_margin_bottom(12);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let folder_entry = Entry::new();
    folder_entry.set_text(&cur.download_dir);
    content.append(&settings_row("Download folder", &folder_entry));

    let conn_spin = gtk4::SpinButton::with_range(1.0, 32.0, 1.0);
    conn_spin.set_value(cur.max_connections as f64);
    content.append(&settings_row("Koneksi per server", &conn_spin));

    let conc_spin = gtk4::SpinButton::with_range(1.0, 10.0, 1.0);
    conc_spin.set_value(cur.max_concurrent as f64);
    content.append(&settings_row("Download bersamaan (antrian)", &conc_spin));

    let speed_entry = Entry::new();
    speed_entry.set_text(&cur.max_overall_speed);
    speed_entry.set_placeholder_text(Some("0 = unlimited, mis. 512K / 2M"));
    content.append(&settings_row("Speed limit total", &speed_entry));

    let verify_tls_chk = gtk4::CheckButton::with_label("Verifikasi sertifikat TLS (aman)");
    verify_tls_chk.set_active(cur.verify_tls);
    content.append(&verify_tls_chk);

    // Buttons
    let btn_box = GtkBox::new(Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = Button::with_label("Batal");
    cancel_btn.add_css_class("btn-action");
    cancel_btn.add_css_class("btn-cancel");

    let save_btn = Button::with_label("Simpan");
    save_btn.add_css_class("btn-download");

    let dialog_weak = dialog.downgrade();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.response(gtk4::ResponseType::Cancel);
        }
    });

    let dialog_weak = dialog.downgrade();
    save_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.response(gtk4::ResponseType::Ok);
        }
    });

    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    content.append(&btn_box);

    dialog.show();

    // Pola nested main loop yang sama dengan youtube_dialog.rs
    let main_context = glib::MainContext::default();
    let response = std::rc::Rc::new(std::cell::RefCell::new(gtk4::ResponseType::Cancel));
    let responded_flg = std::rc::Rc::new(std::cell::RefCell::new(false));

    let rv = response.clone();
    let rf = responded_flg.clone();
    dialog.connect_response(move |d, resp| {
        *rv.borrow_mut() = resp;
        *rf.borrow_mut() = true;
        d.close();
    });

    // Jangan menggantung kalau dialog ditutup lewat tombol close window
    let rv2 = response.clone();
    let rf2 = responded_flg.clone();
    dialog.connect_close_request(move |_| {
        if !*rf2.borrow() {
            *rv2.borrow_mut() = gtk4::ResponseType::Cancel;
        }
        glib::Propagation::Proceed
    });

    while dialog.is_visible() {
        main_context.iteration(true);
    }

    if *response.borrow() != gtk4::ResponseType::Ok {
        return None;
    }

    let mut cfg = cur.clone();
    let dir = folder_entry.text().trim().to_string();
    if !dir.is_empty() {
        cfg.download_dir = dir;
    }
    cfg.max_connections = conn_spin.value() as u8;
    cfg.max_concurrent = conc_spin.value() as u8;
    let speed = speed_entry.text().trim().to_string();
    if !speed.is_empty() {
        cfg.max_overall_speed = speed;
    }
    cfg.verify_tls = verify_tls_chk.is_active();
    Some(cfg)
}
