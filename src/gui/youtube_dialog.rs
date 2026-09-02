use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Dialog, Label, Orientation, ResponseType, ScrolledWindow,
    Window,
};

#[allow(dead_code)]
pub struct QualityOption {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

#[allow(dead_code)]
pub const QUALITIES: &[QualityOption] = &[
    QualityOption {
        id: "best_mp4",
        label: "Best Quality (MP4)",
        desc: "Highest resolution",
    },
    QualityOption {
        id: "2160p",
        label: "4K Ultra HD",
        desc: "3840x2160",
    },
    QualityOption {
        id: "1440p",
        label: "2K QHD",
        desc: "2560x1440",
    },
    QualityOption {
        id: "1080p",
        label: "1080p Full HD",
        desc: "1920x1080",
    },
    QualityOption {
        id: "720p",
        label: "720p HD",
        desc: "1280x720",
    },
    QualityOption {
        id: "480p",
        label: "480p SD",
        desc: "854x480",
    },
    QualityOption {
        id: "360p",
        label: "360p Low",
        desc: "640x360",
    },
    QualityOption {
        id: "audio_best",
        label: "Audio M4A",
        desc: "Best quality",
    },
    QualityOption {
        id: "audio_mp3",
        label: "Audio MP3",
        desc: "320kbps",
    },
];

/// Dialog pemilihan kualitas ala IDM.
///
/// v2.3.2 (M4): pola callback event-driven — `on_ok(String)` dipanggil dari
/// sinyal `response` saat user menekan "Unduh"; "Batal"/tutup = tidak ada
/// aksi. Pola lama (`-> Option<String>` + `while dialog.is_visible() {
/// main_context.iteration(true) }`) adalah NESTED main loop: berisiko
/// reentrancy, dan punya bug nyata — dialog di-Cancel tetap melanjutkan
/// download (return None dianggap "tanpa kualitas"), kini dibatalkan benar.
#[allow(dead_code)]
pub fn show_quality_dialog<F>(
    parent: &Window,
    title: &str,
    uploader: &str,
    duration_str: &str,
    formats: Vec<crate::downloader::youtube::FormatOption>,
    on_ok: F,
) where
    F: FnOnce(String) + 'static,
{
    let dialog = Dialog::with_buttons(
        Some("Pilih Kualitas Video"),
        Some(parent),
        gtk4::DialogFlags::MODAL | gtk4::DialogFlags::DESTROY_WITH_PARENT,
        &[],
    );
    // B2: samakan tema dialog dengan window utama
    dialog.add_css_class("fast-dm-window");
    dialog.set_default_size(480, 460);

    let content = dialog.content_area();
    content.set_spacing(10);
    content.set_margin_top(16);
    content.set_margin_bottom(12);
    content.set_margin_start(20);
    content.set_margin_end(20);

    // Title
    let title_lbl = Label::new(Some(title));
    title_lbl.set_halign(gtk4::Align::Start);
    title_lbl.set_wrap(true);
    title_lbl.set_max_width_chars(50);
    title_lbl.add_css_class("filename-label");
    content.append(&title_lbl);

    // Info — A4: URL sumber (uploader/durasi belum tersedia di tahap ini),
    // dengan ellipsize agar URL panjang tidak melebar.
    let info_box = GtkBox::new(Orientation::Horizontal, 12);
    if !uploader.is_empty() {
        let up = Label::new(Some(uploader));
        up.set_halign(gtk4::Align::Start);
        up.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        up.set_max_width_chars(48);
        up.set_wrap(true);
        up.add_css_class("detail-label");
        info_box.append(&up);
    }
    if !duration_str.is_empty() {
        let dur = Label::new(Some(duration_str));
        dur.add_css_class("detail-label");
        info_box.append(&dur);
    }
    content.append(&info_box);
    content.append(&gtk4::Separator::new(Orientation::Horizontal));

    // Quality header
    let q_label = Label::new(Some("Kualitas"));
    q_label.set_halign(gtk4::Align::Start);
    q_label.add_css_class("filename-label");
    content.append(&q_label);

    // Quality radio buttons
    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(200);

    let quality_box = GtkBox::new(Orientation::Vertical, 4);
    let selected = std::rc::Rc::new(std::cell::RefCell::new("best_mp4".to_string()));

    let mut first_btn: Option<CheckButton> = None;

    for q in QUALITIES {
        let is_default = first_btn.is_none();
        let (radio, row) = quality_row(
            &selected,
            first_btn.as_ref(),
            q.id,
            q.label,
            q.desc,
            is_default,
        );
        if is_default {
            first_btn = Some(radio.clone());
        }
        quality_box.append(&row);
    }

    // v2.6.0 (D6): format NYATA dari situs (hasil `yt-dlp -J` yang diambil
    // window.rs sebelum dialog dibuka). Kosong = fetch gagal/tidak diminta →
    // dialog persis seperti versi sebelumnya (hanya preset).
    if !formats.is_empty() {
        let hdr = Label::new(Some(&format!(
            "Format lengkap dari situs ({}):",
            formats.len()
        )));
        hdr.set_halign(gtk4::Align::Start);
        hdr.set_margin_top(8);
        hdr.add_css_class("detail-label");
        quality_box.append(&hdr);
        for f in &formats {
            let (radio, row) = quality_row(
                &selected,
                first_btn.as_ref(),
                &f.id,
                &f.label,
                &f.desc,
                false,
            );
            if first_btn.is_none() {
                first_btn = Some(radio.clone());
            }
            quality_box.append(&row);
        }
    }

    scroll.set_child(Some(&quality_box));
    content.append(&scroll);

    // Buttons
    let btn_box = GtkBox::new(Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = Button::with_label("Batal");
    cancel_btn.add_css_class("btn-action");
    cancel_btn.add_css_class("btn-cancel");

    let dl_btn = Button::with_label("Unduh");
    dl_btn.add_css_class("btn-download");

    let dialog_weak = dialog.downgrade();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.response(ResponseType::Cancel);
        }
    });

    let dialog_weak = dialog.downgrade();
    dl_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.response(ResponseType::Ok);
        }
    });

    btn_box.append(&cancel_btn);
    btn_box.append(&dl_btn);
    content.append(&btn_box);

    // v2.3.2 (M4): tanpa nested main loop — keputusan user ditangani sinyal
    // `response`; tidak ada loop yang bisa menggantung, jadi guard
    // connect_close_request pola lama tidak diperlukan lagi.
    let sel_result = selected.clone();
    let on_ok = std::rc::Rc::new(std::cell::RefCell::new(Some(on_ok)));
    dialog.connect_response(move |d, resp| {
        if resp == ResponseType::Ok {
            let q = sel_result.borrow().clone();
            if let Some(f) = on_ok.borrow_mut().take() {
                f(q);
            }
        }
        d.close();
    });
    dialog.show();
}

/// Satu baris dialog: radio (se-group) + nama + detail. TEKS BIASA, bukan
/// markup Pango — sejak D6 data label/desc bisa berasal dari output yt-dlp
/// (data halaman!) dan tidak boleh diinterpretasikan sebagai markup.
fn quality_row(
    selected: &std::rc::Rc<std::cell::RefCell<String>>,
    first: Option<&CheckButton>,
    id: &str,
    label_txt: &str,
    desc: &str,
    default_active: bool,
) -> (CheckButton, GtkBox) {
    let radio = match first {
        Some(f) => {
            let r = CheckButton::new();
            r.set_group(Some(f));
            r
        }
        None => CheckButton::new(),
    };
    if default_active {
        radio.set_active(true);
    }
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let label = Label::new(Some(label_txt));
    label.set_halign(gtk4::Align::Start);
    row.append(&radio);
    row.append(&label);
    if !desc.is_empty() {
        let d = Label::new(Some(desc));
        d.add_css_class("detail-label");
        row.append(&d);
    }
    let sel = selected.clone();
    let owned = id.to_string();
    radio.connect_toggled(move |btn| {
        if btn.is_active() {
            *sel.borrow_mut() = owned.clone();
        }
    });
    (radio, row)
}
