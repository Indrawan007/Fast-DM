use crate::downloader::types::*;
use crate::downloader::youtube;
use crate::downloader::DownloadEngine;
use crate::gui::css;
use crate::gui::download_row::DownloadRow;

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
    rt: tokio::runtime::Handle,
) {
    // Load CSS
    let provider = CssProvider::new();
    provider.load_from_string(css::THEME_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Fast Download Manager")
        .default_width(780)
        .default_height(580)
        .build();

    // Set icon
    let icon_paths = [
        "/opt/fast-dm/fast-dm-icon.png",
        "/opt/fast-dm/extension/icons/icon128.png",
    ];
    for path in &icon_paths {
        if std::path::Path::new(path).exists() {
            if let Ok(tex) = gtk4::gdk::Texture::from_filename(path) {
                window.set_icon_name(Some("fast-dm"));
            }
            break;
        }
    }

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

    toolbar.append(&url_entry);
    toolbar.append(&add_btn);
    toolbar.append(&clear_btn);
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

        let eng = engine_add.clone();
        let rt = rt_add.clone();

        glib::spawn_future_local(async move {
            let _ = rt.spawn(async move {
                eng.add_download(&url, None, None, true).await
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

    // ── Event listener ──
    let rows_ev = rows.clone();
    let listbox_ev = listbox.clone();
    let engine_ev = engine.clone();
    let rt_ev = rt.clone();
    let stats_a = stats_active.clone();
    let stats_s = stats_speed.clone();
    let stats_t = stats_total.clone();
    let statuses_ev = download_statuses.clone();

    glib::spawn_future_local(async move {
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

            // Update stats
            drop(rows_map);
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
    });

    window.present();
}
