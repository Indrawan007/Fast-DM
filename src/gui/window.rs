use crate::config::Config;
use crate::downloader::types::*;
use crate::downloader::DownloadEngine;
use crate::gui::css;
use crate::gui::download_row::DownloadRow;
use crate::gui::youtube_dialog;

use glib;
use gtk4::gdk::{ContentFormats, DragAction};
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DropTarget, Entry,
    EventControllerKey, Label, ListBox, Orientation, PolicyType, ScrolledWindow, SelectionMode,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(deprecated)] // gtk4::Dialog (dialog konfirmasi tutup) — dipakai demi pola lama yang sudah ada
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

    // B5: subtitle berisi info berguna → versi aplikasi
    // (dinamis dari Cargo.toml — tidak lagi hardcoded sehingga tidak
    // pernah basi saat versi naik)
    let subtitle = Label::new(Some(&format!("v{}", env!("CARGO_PKG_VERSION"))));
    subtitle.add_css_class("header-subtitle");
    subtitle.set_valign(gtk4::Align::End);

    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    // ── Toolbar ──
    let toolbar = GtkBox::new(Orientation::Horizontal, 8);
    toolbar.add_css_class("toolbar-box");

    // B1: satu bahasa (Indonesia) di seluruh UI
    let url_entry = Entry::new();
    url_entry.set_placeholder_text(Some("Tempel URL unduhan di sini..."));
    url_entry.set_hexpand(true);
    url_entry.add_css_class("url-entry");

    let add_btn = Button::with_label("Unduh");
    add_btn.add_css_class("btn-download");

    // v2.5.0 (D2): dialog "Simpan Sebagai…" ala IDM — tentukan folder & nama
    // file sebelum unduhan mulai.
    let save_as_btn = Button::with_label("Simpan Sebagai…");
    save_as_btn.add_css_class("btn-clear");
    save_as_btn.set_tooltip_text(Some(
        "Pilih folder & nama file sebelum mengunduh — untuk URL video, dialog \
         kualitas tetap menyusul",
    ));

    let pause_all_btn = Button::with_label("Jeda Semua");
    pause_all_btn.add_css_class("btn-clear");
    pause_all_btn.set_tooltip_text(Some("Jeda semua unduhan aktif / lanjutkan semua"));
    pause_all_btn.set_sensitive(false); // di-enable oleh event listener saat ada unduhan

    let clear_btn = Button::with_label("Bersihkan Selesai");
    clear_btn.add_css_class("btn-clear");

    let settings_btn = Button::with_label("Pengaturan");
    settings_btn.add_css_class("btn-clear");

    toolbar.append(&url_entry);
    toolbar.append(&add_btn);
    toolbar.append(&save_as_btn);
    toolbar.append(&pause_all_btn);
    toolbar.append(&clear_btn);
    toolbar.append(&settings_btn);
    root.append(&toolbar);

    // ── v2.4.0 (D1): banner URL clipboard (ala IDM) ──────────────────────
    // Clipboard memuat URL http(s) → tampilkan bar di bawah toolbar dengan
    // opsi "Unduh"; "✕" menutup. Default OFF — diaktifkan lewat Pengaturan;
    // butuh wl-clipboard (Wayland) atau xclip (X11).
    let clip_banner = GtkBox::new(Orientation::Horizontal, 8);
    clip_banner.add_css_class("clipboard-banner");
    clip_banner.set_margin_start(12);
    clip_banner.set_margin_end(12);
    clip_banner.set_margin_top(6);
    clip_banner.set_visible(false);

    let clip_lbl = Label::new(None);
    clip_lbl.set_hexpand(true);
    clip_lbl.set_halign(gtk4::Align::Start);
    clip_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    let clip_dl_btn = Button::with_label("Unduh");
    clip_dl_btn.add_css_class("btn-download");
    let clip_x_btn = Button::with_label("✕");
    clip_x_btn.add_css_class("btn-clear");
    clip_banner.append(&clip_lbl);
    clip_banner.append(&clip_dl_btn);
    clip_banner.append(&clip_x_btn);
    root.append(&clip_banner);

    let clip_pending: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let clip_enabled: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));
    let clip_last: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    // None = belum diprobe; Some(x) = hasil probe (x: Option<&'static str>)
    let clip_tool: Rc<RefCell<Option<Option<&'static str>>>> = Rc::new(RefCell::new(None));

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
    let ph_title = Label::new(Some("Belum ada unduhan"));
    ph_title.add_css_class("ph-title");
    let ph_sub = Label::new(Some("Tempel URL di atas atau pakai ekstensi browser"));
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

    // B3: statistik memisahkan aktif vs antrian
    let stats_active = Label::new(Some("Aktif 0"));
    stats_active.add_css_class("stats-value");
    let stats_speed = Label::new(Some("0 B/s"));
    stats_speed.add_css_class("stats-speed");
    let stats_queued = Label::new(Some("Antri 0"));
    stats_queued.add_css_class("stats-value");
    let stats_total = Label::new(Some("Total 0"));
    stats_total.add_css_class("stats-value");

    statsbar.append(&stats_active);
    statsbar.append(&stats_speed);
    statsbar.append(&stats_queued);
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
    let rows: Rc<RefCell<HashMap<String, DownloadRow>>> = Rc::new(RefCell::new(HashMap::new()));

    // Track download status for clear done
    let download_statuses: Rc<RefCell<HashMap<String, DownloadStatus>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // ── Add URL handler ──
    let engine_add = engine.clone();
    let rt_add = rt.clone();
    let entry_add = url_entry.clone();

    let win_add = window.clone();

    let on_add = move || {
        let raw = entry_add.text().to_string();
        if raw.trim().is_empty() {
            return;
        }

        // v2.5.0 (D2): normalisasi & keputusan dialog kualitas di-ekstrak ke
        // fungsi modul — dipakai bersama dengan tombol "Simpan Sebagai…".
        let url = normalize_url_input(&raw);

        entry_add.set_text("");

        // YouTube + halaman video lain: minta pilihan kualitas langsung dari GUI
        // (IDM-like). File langsung (mp4/zip/dll) dilewati — tidak perlu dialog.
        // (B20: hanya URL yang memang lewat jalur yt-dlp — YouTube, manifest
        // HLS/DASH, wrapper halaman, atau URL tanpa ekstensi. Logika persis di
        // wants_quality_dialog.)
        let wants_quality = wants_quality_dialog(&url);
        let eng = engine_add.clone();
        let rt = rt_add.clone();

        // v2.3.2 (M4): mulai unduhan — dipakai dua jalur (dialog kualitas via
        // callback, atau langsung tanpa dialog). url sengaja di-clone dulu:
        // closure `start` adalah Fn yang bisa dipanggil berkali-kali, jadi
        // ia memegang SALINAN (url_for_start), bukan url aslinya (E0382).
        let url_for_start = url.clone();
        let start = move |quality: Option<String>| {
            let eng = eng.clone();
            let rt = rt.clone();
            let url = url_for_start.clone();
            glib::spawn_future_local(async move {
                let _ = rt
                    .spawn(async move {
                        eng.add_download(&url, None, None, true, Default::default(), quality)
                            .await
                    })
                    .await;
            });
        };

        if wants_quality {
            // Cancel/tutup dialog = batal total (dulu lanjut tanpa kualitas).
            //
            // v2.6.0 (D6): ambil format NYATA dulu (yt-dlp -J, cap internal
            // 20 dtk) — dialog menyusul menampilkannya; fetch gagal → list
            // kosong → dialog hanya berisi preset (perilaku ≤2.5.x).
            let engine_q = engine_add.clone();
            let rt_q = rt_add.clone();
            let url_q = url.clone();
            let win_q = win_add.clone();
            let start_q = start.clone();
            glib::spawn_future_local(async move {
                let fmts = {
                    let eng = engine_q.clone();
                    let cfg = rt_q
                        .spawn(async move { eng.get_config().await })
                        .await
                        .unwrap_or_default();
                    let url2 = url_q.clone();
                    rt_q.spawn(async move {
                        crate::downloader::youtube::fetch_formats(&url2, &cfg).await
                    })
                    .await
                    .unwrap_or_default()
                };
                youtube_dialog::show_quality_dialog(
                    win_q.upcast_ref(),
                    "Pilih kualitas",
                    &url_q,
                    "",
                    fmts,
                    move |q| start_q(Some(q)),
                );
            });
        } else {
            start(None);
        }
    };

    let on_add_clone = on_add.clone();
    add_btn.connect_clicked(move |_| on_add_clone());

    let on_add_clone = on_add.clone();
    url_entry.connect_activate(move |_| on_add_clone());

    // ── v2.4.0 (D1): wiring clipboard + polling ──
    {
        let banner = clip_banner.clone();
        let pend = clip_pending.clone();
        let entry = url_entry.clone();
        let on_add_clip = on_add.clone();
        clip_dl_btn.connect_clicked(move |_| {
            if let Some(u) = pend.borrow_mut().take() {
                entry.set_text(&u);
                banner.set_visible(false);
                // Alur sama persis dengan input manual: on_add membaca Entry.
                on_add_clip();
            }
        });

        let banner_x = clip_banner.clone();
        let pend_x = clip_pending.clone();
        clip_x_btn.connect_clicked(move |_| {
            *pend_x.borrow_mut() = None;
            banner_x.set_visible(false);
        });

        // Nilai awal = config tersimpan (block_on aman di main thread —
        // pola sama seperti handler settings).
        {
            let eng = engine.clone();
            let cfg0 = rt.block_on(async move { eng.get_config().await });
            clip_enabled.set(cfg0.clipboard_monitor);
        }

        // Polling (bukan listener): tanpa dependensi baru. Jeda 2.5 dtk;
        // saat toggle OFF biayanya cuma cek boolean.
        let en = clip_enabled.clone();
        let lbl_t = clip_lbl.clone();
        let ban_t = clip_banner.clone();
        let pend_t = clip_pending.clone();
        let last_t = clip_last.clone();
        let tool_t = clip_tool.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(2500), move || {
            if !en.get() {
                if ban_t.is_visible() {
                    ban_t.set_visible(false);
                }
                return glib::ControlFlow::Continue;
            }
            let tool: Option<&'static str> = {
                let mut tr = tool_t.borrow_mut();
                let v = tr.get_or_insert_with(clipboard_probe);
                *v
            };
            let Some(tool) = tool else {
                // tak ada wl-paste/xclip → stop polling (jangan buang timer)
                return glib::ControlFlow::Break;
            };
            let Some(txt) = clipboard_text(tool) else {
                return glib::ControlFlow::Continue;
            };
            // Dedup: konten sama (termasuk non-URL) tidak diulang-ulang.
            if txt.is_empty() || txt == *last_t.borrow() {
                return glib::ControlFlow::Continue;
            }
            *last_t.borrow_mut() = txt.clone();
            if txt.len() > 2048 || !(txt.starts_with("http://") || txt.starts_with("https://")) {
                if ban_t.is_visible() {
                    ban_t.set_visible(false);
                }
                return glib::ControlFlow::Continue;
            }
            *pend_t.borrow_mut() = Some(txt.clone());
            lbl_t.set_text(&format!("📋 URL terdeteksi di clipboard: {}", txt));
            ban_t.set_visible(true);
            glib::ControlFlow::Continue
        });
    }

    // ── v2.5.0 (D2): Simpan Sebagai… — folder + nama file dulu, baru unduh ──
    {
        let win_ref = window.clone();
        let engine_ref = engine.clone();
        let rt_ref = rt.clone();
        let entry_sa = url_entry.clone();
        save_as_btn.connect_clicked(move |_| {
            let raw = entry_sa.text().to_string();
            if raw.trim().is_empty() {
                // Paritas dengan tombol Unduh: kosong → tidak ada aksi
                return;
            }
            let url = normalize_url_input(&raw);
            let suggested = crate::downloader::extract_filename_from_url(&url);
            // Folder awal = download_dir dari config (block_on aman di main
            // thread — pola sama seperti handler settings/clipboard).
            let eng0 = engine_ref.clone();
            let cfg = rt_ref.block_on(async move { eng0.get_config().await });
            let start_dir = cfg.download_dir.clone();

            let dlg = gtk4::FileDialog::builder()
                .title("Simpan sebagai…")
                .accept_label("Simpan")
                .initial_name(suggested.as_str())
                .initial_folder(&gtk4::gio::File::for_path(&start_dir))
                .modal(true)
                .build();

            // v2.5.1: gtk4-rs 0.9 menamai method callback FileDialog cukup
            // `save()` — pola identik dengan `select_folder` di dialog settings
            // (nama `save_file` adalah alias dokumentasi C, bukan method Rust;
            // `save_future()` juga ada, tapi callback lebih konsisten di sini).
            // win_parent = klon utk argumen &-borrow; dialog kualitas memakai
            // klon sendiri (hindari borrow+move variabel sama satu expression).
            let win_parent = win_ref.clone();
            let win_cb = win_ref.clone();
            let engine_cb = engine_ref.clone();
            let rt_cb = rt_ref.clone();
            let entry_cb = entry_sa.clone();
            dlg.save(
                Some(&win_parent),
                gtk4::gio::Cancellable::NONE,
                move |res| {
                    let Ok(file) = res else { return }; // batal → tidak ada aksi
                    let Some(path) = file.path() else { return };
                    let Some(fname) = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                    else {
                        return;
                    };
                    let dir = path
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or(start_dir);
                    entry_cb.set_text(""); // UI bersih seperti alur Unduh normal

                    // v2.3.2 lesson (E0382/'static): closure `start` disimpan
                    // dialog kualitas sampai respons user tiba — wajib memiliki
                    // datanya sendiri (salinan), bukan meminjam lokal scope ini.
                    let eng = engine_cb.clone();
                    let rt = rt_cb.clone();
                    let url_for_start = url.clone();
                    let start = move |quality: Option<String>| {
                        let eng = eng.clone();
                        let rt = rt.clone();
                        let url = url_for_start.clone();
                        glib::spawn_future_local(async move {
                            let _ = rt
                                .spawn(async move {
                                    eng.add_download(
                                        &url,
                                        Some(&fname),
                                        Some(&dir),
                                        true,
                                        Default::default(),
                                        quality,
                                    )
                                    .await
                                })
                                .await;
                        });
                    };

                    if wants_quality_dialog(&url) {
                        // v2.6.0: jalur save-as memakai preset statis (tanpa
                        // fetch) — dialog file-nya saja sudah dua langkah.
                        youtube_dialog::show_quality_dialog(
                            win_cb.upcast_ref(),
                            "Pilih kualitas",
                            &url,
                            "",
                            Vec::new(),
                            move |q| start(Some(q)),
                        );
                    } else {
                        start(None);
                    }
                },
            );
        });
    }

    // ── C2: drag & drop URL (dari browser/file manager) ke window ──
    let drop_target = DropTarget::builder()
        .actions(DragAction::COPY)
        .formats(&ContentFormats::new(&["text/uri-list", "text/plain"]))
        .build();
    window.add_controller(drop_target.clone());

    let entry_drop = url_entry.clone();
    let on_add_drop = on_add.clone();
    drop_target.connect_drop(move |_, value, _x, _y| {
        let mut urls: Vec<String> = Vec::new();
        if let Ok(text) = value.get::<String>() {
            for line in text.lines() {
                let uri = line.trim();
                if uri.is_empty()
                    || uri.starts_with('#') // baris komentar text/uri-list
                    || uri.starts_with("file://")
                {
                    continue; // file lokal bukan URL yang bisa diunduh
                }
                urls.push(uri.to_string());
            }
        }
        if urls.is_empty() {
            return false; // tidak ada URL → jangan klaim drop
        }

        // Tunda ke idle: on_add bisa memunculkan dialog modal (YouTube) —
        // tetap ditunda agar sinyal drop selesai cepat (v2.3.2/M4: nested
        // main loop sudah dihapus, tapi defer untuk daftar URL multi tetap baik).
        let entry = entry_drop.clone();
        let add = on_add_drop.clone();
        glib::idle_add_local_once(move || {
            for u in urls {
                entry.set_text(&u);
                add();
            }
        });
        true
    });

    // ── C3: shortcut keyboard Ctrl+L → fokus input URL ──
    let key_ctrl = EventControllerKey::new();
    let entry_ctrl = url_entry.clone();
    key_ctrl.connect_key_pressed(move |_, key, _code, mods| {
        if mods.contains(gtk4::gdk::ModifierType::CONTROL_MASK) && key == gtk4::gdk::Key::l {
            entry_ctrl.grab_focus();
            entry_ctrl.select_region(0, -1);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_ctrl);

    // ── Clear Done handler ──
    let rows_clear = rows.clone();
    let listbox_clear = listbox.clone();
    let engine_clear = engine.clone();
    let rt_clear = rt.clone();
    let statuses_clear = download_statuses.clone();

    clear_btn.connect_clicked(move |_| {
        let statuses = statuses_clear.borrow();
        let to_remove: Vec<String> = statuses
            .iter()
            .filter(|(_, status)| {
                matches!(
                    status,
                    DownloadStatus::Completed | DownloadStatus::Cancelled | DownloadStatus::Error
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
                let _ = rt
                    .spawn(async move {
                        eng.clear_download(&id_for_engine).await;
                    })
                    .await;
            });
        }

        // Remove from status tracking (setelah loop selesai)
        let mut st = statuses_clear.borrow_mut();
        for id in &to_remove {
            st.remove(id);
        }
    });

    // ── C3: Jeda Semua / Lanjut Semua ──
    // state=false → masih ada yang aktif (aksi = Jeda); state=true → aksi = Lanjut.
    // Label & state disinkronkan ulang oleh event listener (state nyata), bukan hanya klik.
    let pause_all_state = Rc::new(Cell::new(false));
    let pause_all_btn_h = pause_all_btn.clone();
    let engine_pa = engine.clone();
    let rt_pa = rt.clone();
    let state_pa = pause_all_state.clone();
    pause_all_btn.connect_clicked(move |_| {
        let eng = engine_pa.clone();
        let rt = rt_pa.clone();
        let state = state_pa.clone();
        let btn = pause_all_btn_h.clone();
        glib::spawn_future_local(async move {
            let do_pause = !state.get();
            if do_pause {
                let _ = rt.spawn(async move { eng.pause_all().await }).await;
            } else {
                let _ = rt.spawn(async move { eng.resume_all().await }).await;
            }
            // state=false → masih ada yang aktif (aksi berikutnya = Jeda)
            state.set(do_pause);
            btn.set_label(if do_pause {
                "Lanjut Semua"
            } else {
                "Jeda Semua"
            });
        });
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

        // v2.3.2 (M4): dialog kini event-driven — aksi simpan berjalan dari
        // callback saat user menekan "Simpan" (bukan setelah nested loop).
        // Simpan ke disk + apply live; tampilkan feedback (termasuk kalau
        // validasi engine menolak) — semuanya di dalam on_ok.
        let eng2 = engine_settings.clone();
        let rt2 = rt_settings.clone();
        let btn_feedback_src = settings_btn_h.clone();
        let clip_flag = clip_enabled.clone();
        let clip_ban = clip_banner.clone();
        // v2.8.0 (D8.1): side-effect file (autostart) hanya saat nilainya
        // BERUBAH dan hanya setelah engine benar-benar menerima config.
        let prev_autostart = cur.autostart;
        show_settings_dialog(win_settings.upcast_ref(), &cur, move |cfg| {
            // v2.4.0 (D1): toggle clipboard baru berlaku setelah settings
            // BENAR-BENAR tersimpan (engine bisa menolaknya, mis. proxy invalid).
            let want_clip = cfg.clipboard_monitor;
            let want_autostart = cfg.autostart;
            let autostart_changed = want_autostart != prev_autostart;
            let handle = rt2.spawn(async move { eng2.update_config(cfg).await });
            let btn_feedback = btn_feedback_src.clone();
            glib::spawn_future_local(async move {
                match handle.await {
                    Ok(Ok(())) => {
                        btn_feedback.set_label("✓ Tersimpan");
                        clip_flag.set(want_clip);
                        if !want_clip {
                            clip_ban.set_visible(false);
                        }
                        if autostart_changed {
                            if let Err(e) = crate::config::Config::apply_autostart(want_autostart) {
                                tracing::warn!("Autostart: {}", e);
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Settings rejected: {}", e);
                        btn_feedback.set_label("✕ Gagal Simpan");
                    }
                    Err(_) => btn_feedback.set_label("✕ Gagal"),
                }
                let btn = btn_feedback.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                    btn.set_label("Pengaturan");
                    glib::ControlFlow::Break
                });
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
    let stats_q = stats_queued.clone();
    let stats_t = stats_total.clone();
    let statuses_ev = download_statuses.clone();
    let pa_btn = pause_all_btn.clone();
    let pa_state = pause_all_state.clone();

    glib::spawn_future_local(async move {
        let mut last_stats = std::time::Instant::now();
        while let Some(event) = event_rx.recv().await {
            let info = match &event {
                DownloadEvent::Progress(i) => i,
                DownloadEvent::Completed(i) => i,
                DownloadEvent::Error(i) => i,
            };

            // Track status
            statuses_ev
                .borrow_mut()
                .insert(info.id.clone(), info.status);

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
                        let _ = rt.spawn(async move { eng.pause_download(&id).await }).await;
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
                        let _ = rt
                            .spawn(async move { eng.resume_download(&id).await })
                            .await;
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
                        let _ = rt
                            .spawn(async move { eng.cancel_download(&id).await })
                            .await;
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
                        let _ = rt
                            .spawn(async move { eng.resume_download(&id).await })
                            .await;
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
                        let _ = rt
                            .spawn(async move {
                                eng.clear_download(&id).await;
                            })
                            .await;
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
                let sq = stats_q.clone();
                let st = stats_t.clone();
                let pa_btn_l = pa_btn.clone();
                let pa_state_l = pa_state.clone();

                glib::spawn_future_local(async move {
                    if let Ok(all) = rt.spawn(async move { eng.get_all_downloads().await }).await {
                        // "Aktif" = mengunduh ATAU memproses (resolving) —
                        // konsisten dengan logika slot engine (keduanya
                        // menempati slot download bersamaan).
                        let active: Vec<_> = all
                            .iter()
                            .filter(|d| {
                                matches!(
                                    d.status,
                                    DownloadStatus::Downloading | DownloadStatus::Resolving
                                )
                            })
                            .collect();
                        let queued = all
                            .iter()
                            .filter(|d| matches!(d.status, DownloadStatus::Queued))
                            .count();
                        let total_speed: u64 = active.iter().map(|d| d.speed).sum();

                        sa.set_text(&format!("Aktif {}", active.len()));
                        sq.set_text(&format!("Antri {}", queued));
                        ss.set_text(&if total_speed > 0 {
                            format!("{}/s", format_size(total_speed))
                        } else {
                            "0 B/s".to_string()
                        });
                        st.set_text(&format!("Total {}", all.len()));

                        // Sinkron tombol Jeda/Lanjut Semua dengan state NYATA (C3)
                        let has_active = all.iter().any(|d| {
                            matches!(
                                d.status,
                                DownloadStatus::Downloading | DownloadStatus::Resolving
                            )
                        });
                        let has_pausable = all.iter().any(|d| {
                            matches!(d.status, DownloadStatus::Paused | DownloadStatus::Error)
                        });
                        if has_active {
                            pa_state_l.set(false);
                            pa_btn_l.set_label("Jeda Semua");
                        } else if has_pausable {
                            pa_state_l.set(true);
                            pa_btn_l.set_label("Lanjut Semua");
                        } else {
                            pa_state_l.set(false);
                            pa_btn_l.set_label("Jeda Semua");
                        }
                        pa_btn_l.set_sensitive(has_active || has_pausable);
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

    // v2.3.0 (K5): lanjutkan otomatis unduhan yang terputus saat app ditutup
    // — klaim README kini ditepati. Antrian/slot tetap diatur engine
    // (max_concurrent tidak jebol); bisa dimatikan di Pengaturan.
    {
        let eng_resume = engine.clone();
        rt.spawn(async move {
            eng_resume.resume_restored().await;
        });
    }

    // Jaring pengaman untuk exit selain tombol close (mis. session logout).
    // `DownloadEngine::shutdown` idempotent, jadi aman bila close handler di
    // bawah sudah memanggilnya lebih dulu.
    {
        let engine_shutdown = engine.clone();
        let rt_shutdown = rt.clone();
        app.connect_shutdown(move |_| {
            let eng = engine_shutdown.clone();
            rt_shutdown.block_on(async move { eng.shutdown().await });
        });
    }

    // ── A1: tutup jendela TIDAK diam-diam mematikan download ──
    // Jika masih ada unduhan aktif/antri, minta konfirmasi dulu.
    let engine_close = engine.clone();
    let rt_close = rt.clone();
    let win_close = window.clone();
    let close_confirmed = Rc::new(Cell::new(false));
    let close_confirmed_cb = close_confirmed.clone();
    window.connect_close_request(move |_| {
        // Putaran kedua: user sudah konfirmasi → hentikan subprocess DAN
        // daemon RPC, tulis snapshot final, lalu tutup. Engine memakai SIGTERM
        // / forcePause agar file parsial tetap resumable.
        if close_confirmed_cb.get() {
            let eng = engine_close.clone();
            rt_close.block_on(async move { eng.shutdown().await });
            return glib::Propagation::Proceed;
        }

        let eng = engine_close.clone();
        let all = rt_close.block_on(async move { eng.get_all_downloads().await });
        let active: usize = all
            .iter()
            .filter(|d| {
                matches!(
                    d.status,
                    DownloadStatus::Downloading
                        | DownloadStatus::Resolving
                        | DownloadStatus::Queued
                )
            })
            .count();

        if active == 0 {
            // Termasuk membersihkan daemon idle / task RPC paused yang tidak
            // dihitung sebagai aktif oleh dialog konfirmasi.
            let eng = engine_close.clone();
            rt_close.block_on(async move { eng.shutdown().await });
            return glib::Propagation::Proceed;
        }

        // v2.8.0 (D8.1): minimize-to-close (opt-in, default OFF) — jendela
        // disembunyikan, engine tetap jalan. Buka lagi: jalankan ulang
        // `fast-dm` — single-instance (app.rs) mem-forward activate ke proses
        // pertama dan memanggil present(). Config dibaca SAAT klik tutup
        // supaya perubahan di Pengaturan langsung berlaku tanpa restart.
        {
            let eng_cfg = engine_close.clone();
            let cfg_now = rt_close.block_on(async move { eng_cfg.get_config().await });
            if should_minimize_on_close(cfg_now.minimize_to_close, active) {
                win_close.hide();
                return glib::Propagation::Stop;
            }
        }

        let dlg = gtk4::Dialog::with_buttons(
            Some("Konfirmasi"),
            Some(&win_close),
            gtk4::DialogFlags::MODAL | gtk4::DialogFlags::DESTROY_WITH_PARENT,
            &[
                ("Batal", gtk4::ResponseType::Cancel),
                ("Hentikan & Tutup", gtk4::ResponseType::Accept),
            ],
        );
        dlg.add_css_class("fast-dm-window");
        let content = dlg.content_area();
        content.set_spacing(8);
        content.set_margin_top(16);
        content.set_margin_bottom(12);
        content.set_margin_start(20);
        content.set_margin_end(20);

        let lbl = Label::new(Some(&format!(
            "Masih ada {} unduhan aktif.\nHentikan semua proses dan tutup aplikasi?",
            active
        )));
        lbl.set_wrap(true);
        lbl.set_halign(gtk4::Align::Start);
        content.append(&lbl);
        dlg.show();

        let confirmed = close_confirmed_cb.clone();
        let win2 = win_close.clone();
        dlg.connect_response(move |d, resp| {
            if resp == gtk4::ResponseType::Accept {
                confirmed.set(true);
                win2.close(); // memicu close_request lagi → SIGTERM + tutup
            }
            d.close();
        });

        // Jangan tutup dulu — tunggu keputusan user
        glib::Propagation::Stop
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

/// Dialog settings — perubahan berlaku untuk download baru tanpa restart.
/// Validasi INLINE (A2) sebelum dialog ditutup; folder bisa dipilih via dialog (C1).
///
/// v2.3.2 (M4): `on_ok(Config)` dipanggil dari sinyal `response` saat user
/// menekan "Simpan" dan validasi inline lolos. Pola lama (return
/// `Option<Config>` + `while dialog.is_visible() { main_context.iteration }`)
/// adalah nested main loop — rawan reentrancy dan tidak perlu.
#[allow(deprecated)] // gtk4::Dialog deprecated sejak 4.10 — pola sama seperti youtube_dialog.rs
fn show_settings_dialog<F>(parent: &gtk4::Window, cur: &Config, on_ok: F)
where
    F: FnOnce(Config) + 'static,
{
    let dialog = gtk4::Dialog::with_buttons(
        Some("Pengaturan"),
        Some(parent),
        gtk4::DialogFlags::MODAL | gtk4::DialogFlags::DESTROY_WITH_PARENT,
        &[],
    );
    // B2: samakan tema dialog dengan window utama (CSS di-scope ke class ini)
    dialog.add_css_class("fast-dm-window");
    dialog.set_default_size(460, -1);

    let content = dialog.content_area();
    content.set_spacing(10);
    content.set_margin_top(16);
    content.set_margin_bottom(12);
    content.set_margin_start(20);
    content.set_margin_end(20);

    // ── C1: folder unduhan + tombol "Pilih Folder…" ──
    let folder_row = GtkBox::new(Orientation::Horizontal, 8);
    let folder_entry = Entry::new();
    folder_entry.set_text(&cur.download_dir);
    folder_entry.set_hexpand(true);
    let browse_btn = Button::with_label("Pilih Folder…");
    browse_btn.add_css_class("btn-clear");
    browse_btn.set_tooltip_text(Some("Pilih folder tujuan via dialog"));

    let folder_entry_b = folder_entry.clone();
    let parent_chooser = (*parent).clone();
    browse_btn.connect_clicked(move |_| {
        let chooser = gtk4::FileDialog::builder()
            .title("Pilih folder unduhan")
            .accept_label("Pilih")
            .build();
        let entry = folder_entry_b.clone();
        chooser.select_folder(
            Some(&parent_chooser),
            gtk4::gio::Cancellable::NONE,
            move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        entry.set_text(path.to_string_lossy().as_ref());
                    }
                }
            },
        );
    });

    folder_row.append(&folder_entry);
    folder_row.append(&browse_btn);
    content.append(&settings_row("Folder unduhan", &folder_row));

    let conn_spin = gtk4::SpinButton::with_range(1.0, 32.0, 1.0);
    conn_spin.set_value(cur.max_connections as f64);
    content.append(&settings_row("Koneksi per server", &conn_spin));

    let conc_spin = gtk4::SpinButton::with_range(1.0, 10.0, 1.0);
    conc_spin.set_value(cur.max_concurrent as f64);
    content.append(&settings_row("Unduhan bersamaan (antrian)", &conc_spin));

    // ── A3: batas kecepatan + hint format · A2: pesan error inline ──
    let speed_box = GtkBox::new(Orientation::Vertical, 4);
    let speed_entry = Entry::new();
    speed_entry.set_text(&cur.max_overall_speed);
    speed_entry.set_placeholder_text(Some("0, 512K, 2M, 10G"));
    let speed_hint = Label::new(Some("0 = tanpa batas · contoh: 512K, 2M, 10G"));
    speed_hint.add_css_class("detail-label");
    speed_hint.set_halign(gtk4::Align::Start);
    let speed_error = Label::new(Some(""));
    speed_error.add_css_class("error-label");
    speed_error.set_halign(gtk4::Align::Start);
    speed_error.set_wrap(true);
    speed_error.set_visible(false);
    speed_box.append(&speed_entry);
    speed_box.append(&speed_hint);
    speed_box.append(&speed_error);
    content.append(&settings_row("Batas kecepatan total", &speed_box));

    let verify_tls_chk = gtk4::CheckButton::with_label("Verifikasi sertifikat TLS (aman)");
    verify_tls_chk.set_active(cur.verify_tls);
    content.append(&verify_tls_chk);

    // v2.3.0 (K5): toggle auto-resume hasil restore sesi
    let auto_resume_chk =
        gtk4::CheckButton::with_label("Lanjutkan otomatis unduhan tertunda saat aplikasi dibuka");
    auto_resume_chk.set_active(cur.auto_resume);
    content.append(&auto_resume_chk);

    // ── v2.4.0 (D3): proxy untuk semua engine (aria2 --all-proxy ·
    // yt-dlp --proxy). Kredensial boleh di dalam URL proxy.
    let proxy_entry = Entry::new();
    proxy_entry.set_text(&cur.proxy_url);
    proxy_entry.set_placeholder_text(Some(
        "http://127.0.0.1:8080 · socks5://host:1080 — kosong = tanpa proxy",
    ));
    proxy_entry.set_hexpand(true);
    content.append(&settings_row("Proxy", &proxy_entry));

    // v2.4.0 (D1): toggle deteksi clipboard
    let clip_chk = gtk4::CheckButton::with_label(
        "Deteksi URL unduhan dari clipboard (butuh xclip / wl-clipboard)",
    );
    clip_chk.set_active(cur.clipboard_monitor);
    content.append(&clip_chk);

    // ── v2.8.0 (D8): close-behavior + autostart ──
    let minimize_chk = gtk4::CheckButton::with_label(
        "Tetap jalankan di latar saat jendela ditutup (buka lagi: jalankan ulang Fast DM)",
    );
    minimize_chk.set_active(cur.minimize_to_close);
    content.append(&minimize_chk);

    let autostart_chk = gtk4::CheckButton::with_label("Jalankan Fast DM otomatis saat login");
    autostart_chk.set_active(cur.autostart);
    content.append(&autostart_chk);

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

    // A2: validasi INLINE sebelum dialog ditutup — user tahu field mana yang salah
    let dialog_weak = dialog.downgrade();
    let speed_entry_save = speed_entry.clone();
    let speed_error_save = speed_error.clone();
    save_btn.connect_clicked(move |_| {
        let speed = speed_entry_save.text().trim().to_string();
        if !speed.is_empty() && !crate::downloader::is_valid_speed_limit(&speed) {
            speed_error_save.set_text("Format tidak valid — gunakan: 0, 512K, 2M, 10G");
            speed_error_save.set_visible(true);
            return; // dialog tetap terbuka
        }
        speed_error_save.set_visible(false);
        if let Some(d) = dialog_weak.upgrade() {
            d.response(gtk4::ResponseType::Ok);
        }
    });

    // Sembunyikan pesan error begitu user mengetik ulang
    let speed_error_hide = speed_error.clone();
    speed_entry.connect_changed(move |_| {
        speed_error_hide.set_visible(false);
    });

    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    content.append(&btn_box);

    // v2.3.2 (M4): bangun config & panggil on_ok dari sinyal response —
    // tanpa nested main loop. Cancel/tutup jendela = tidak ada aksi (tidak ada
    // loop yang bisa menggantung, jadi guard close_request pola lama hilang).
    let cfg_base = cur.clone();
    let on_ok = std::rc::Rc::new(std::cell::RefCell::new(Some(on_ok)));
    let (fe, cs, cc, se, vt, ar, px, cb, mz, au) = (
        folder_entry.clone(),
        conn_spin.clone(),
        conc_spin.clone(),
        speed_entry.clone(),
        verify_tls_chk.clone(),
        auto_resume_chk.clone(),
        proxy_entry.clone(),
        clip_chk.clone(),
        minimize_chk.clone(),
        autostart_chk.clone(),
    );
    dialog.connect_response(move |d, resp| {
        if resp == gtk4::ResponseType::Ok {
            let mut cfg = cfg_base.clone();
            let dir = fe.text().trim().to_string();
            if !dir.is_empty() {
                cfg.download_dir = dir;
            }
            cfg.max_connections = cs.value() as u8;
            cfg.max_concurrent = cc.value() as u8;
            // Normalisasi: "2m" → "2M", "10g" → "10G"
            let speed = se.text().trim().to_uppercase();
            if !speed.is_empty() {
                cfg.max_overall_speed = speed;
            }
            cfg.verify_tls = vt.is_active();
            cfg.auto_resume = ar.is_active();
            // v2.4.0 (D3/D1)
            cfg.proxy_url = px.text().trim().to_string();
            cfg.clipboard_monitor = cb.is_active();
            // v2.8.0 (D8.1)
            cfg.minimize_to_close = mz.is_active();
            cfg.autostart = au.is_active();
            if let Some(f) = on_ok.borrow_mut().take() {
                f(cfg);
            }
        }
        d.close();
    });
    dialog.show();
}

// ── v2.5.0 (D2): helper input URL — dipakai tombol "Unduh" & "Simpan Sebagai…" ──

/// v2.8.0 (D8.1): keputusan tunggal close-request — minimize hanya bila
/// fiturnya ON DAN ada unduhan hidup (aktif/antri). Nol aktif → tutup biasa
/// (tak ada gunanya app hidup tanpa apa pun berjalan).
pub(crate) fn should_minimize_on_close(minimize: bool, active: usize) -> bool {
    minimize && active > 0
}

/// Normalisasi input user: tanpa skema → asumsikan https (perilaku lama on_add).
pub(crate) fn normalize_url_input(raw: &str) -> String {
    let url = raw.trim().to_string();
    // v2.7.0 (B2): magnet bukan URL web — jangan di-prefix https://
    if url.to_ascii_lowercase().starts_with("magnet:") {
        return url;
    }
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("ftp://") {
        url
    } else {
        format!("https://{}", url)
    }
}

/// Apakah URL ini melewati jalur yt-dlp yang butuh dialog kualitas (B20)?
/// Ekstensi dicek terhadap PATH saja — host selalu mengandung titik, sehingga
/// cek ke string utuh membuat kondisi "tanpa ekstensi" hampir tak terpenuhi.
pub(crate) fn wants_quality_dialog(url: &str) -> bool {
    // v2.7.0 (B2): magnet tidak punya "kualitas" → tanpa dialog
    if url.to_ascii_lowercase().starts_with("magnet:") {
        return false;
    }
    let path_lower = url::Url::parse(url)
        .ok()
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_default();
    const PAGE_OR_STREAM_EXTS: &[&str] = &[
        ".php", ".html", ".htm", ".asp", ".aspx", ".jsp", ".m3u8", ".mpd",
    ];
    let no_ext = path_lower.is_empty() || path_lower == "/" || !path_lower.contains('.');
    let page_or_stream = PAGE_OR_STREAM_EXTS.iter().any(|e| path_lower.ends_with(e)) || no_ext;
    crate::downloader::youtube::is_youtube_url(url) || page_or_stream
}

// ── v2.4.0 (D1): helper clipboard CLI (tanpa dependensi baru) ────────────

/// Preferensi sesuai sesi: Wayland → wl-paste, X11 → xclip. "Tersedia" =
/// perintah BISA dijalankan (spawn tidak NotFound); status exit diabaikan
/// (clipboard kosong pun tetap berarti tool ada).
fn clipboard_probe() -> Option<&'static str> {
    let order: &[&'static str] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        &["wl-paste", "xclip"]
    } else {
        &["xclip", "wl-paste"]
    };
    for bin in order {
        let args: &[&str] = if *bin == "wl-paste" {
            &["--version"]
        } else {
            &["-version"]
        };
        match std::process::Command::new(*bin).args(args).output() {
            Ok(_) => return Some(bin),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        }
    }
    None
}

/// Teks clipboard (None = gagal baca / tidak ada pemilik seleksi).
fn clipboard_text(tool: &'static str) -> Option<String> {
    let out = match tool {
        "wl-paste" => std::process::Command::new("wl-paste")
            .args(["--no-newline"])
            .output(),
        _ => std::process::Command::new("xclip")
            .args(["-o", "-selection", "clipboard"])
            .output(),
    };
    out.ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimize_only_when_on_and_active() {
        assert!(should_minimize_on_close(true, 1));
        assert!(!should_minimize_on_close(false, 5)); // off → dialog lama
        assert!(!should_minimize_on_close(true, 0)); // tak ada yang dikerjakan
    }

    #[test]
    fn normalize_url_passes_magnet_through() {
        assert_eq!(
            normalize_url_input("  magnet:?xt=urn:btih:ab12 "),
            "magnet:?xt=urn:btih:ab12"
        );
        assert_eq!(normalize_url_input("MAGNET:?xt=x"), "MAGNET:?xt=x");
    }

    #[test]
    fn wants_quality_skips_magnet() {
        assert!(!wants_quality_dialog("magnet:?xt=urn:btih:deadbeef"));
    }

    #[test]
    fn normalize_url_adds_https_when_scheme_missing() {
        assert_eq!(
            normalize_url_input("example.com/a.zip"),
            "https://example.com/a.zip"
        );
        assert_eq!(normalize_url_input("  http://x/y  "), "http://x/y");
        assert_eq!(normalize_url_input("ftp://h/f"), "ftp://h/f");
    }

    #[test]
    fn wants_quality_skips_direct_files() {
        assert!(!wants_quality_dialog("https://cdn.example.com/movie.mp4"));
        assert!(!wants_quality_dialog(
            "https://example.com/tool.zip?x=1#frag"
        ));
    }

    #[test]
    fn wants_quality_allows_pages_streams_youtube() {
        assert!(wants_quality_dialog("https://vimeo.com/12345")); // tanpa ekstensi
        assert!(wants_quality_dialog("https://site.com/live/master.m3u8"));
        assert!(wants_quality_dialog(
            "https://www.youtube.com/watch?v=abcdefghijk"
        ));
        assert!(wants_quality_dialog("https://example.com/page.php"));
    }
}
