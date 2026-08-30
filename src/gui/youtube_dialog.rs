use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, CheckButton, Dialog, Label,
    Orientation, ResponseType, ScrolledWindow, Window,
};

#[allow(dead_code)]
pub struct QualityOption {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

#[allow(dead_code)]
pub const QUALITIES: &[QualityOption] = &[
    QualityOption { id: "best_mp4", label: "Best Quality (MP4)",  desc: "Highest resolution" },
    QualityOption { id: "2160p",    label: "4K Ultra HD",         desc: "3840x2160" },
    QualityOption { id: "1440p",    label: "2K QHD",              desc: "2560x1440" },
    QualityOption { id: "1080p",    label: "1080p Full HD",       desc: "1920x1080" },
    QualityOption { id: "720p",     label: "720p HD",             desc: "1280x720" },
    QualityOption { id: "480p",     label: "480p SD",             desc: "854x480" },
    QualityOption { id: "360p",     label: "360p Low",            desc: "640x360" },
    QualityOption { id: "audio_best", label: "Audio M4A",         desc: "Best quality" },
    QualityOption { id: "audio_mp3",  label: "Audio MP3",         desc: "320kbps" },
];

#[allow(dead_code)]
pub fn show_quality_dialog(
    parent: &Window,
    title: &str,
    uploader: &str,
    duration_str: &str,
) -> Option<String> {
    let dialog = Dialog::with_buttons(
        Some("YouTube Download"),
        Some(parent),
        gtk4::DialogFlags::MODAL | gtk4::DialogFlags::DESTROY_WITH_PARENT,
        &[],
    );
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

    // Info
    let info_box = GtkBox::new(Orientation::Horizontal, 12);
    if !uploader.is_empty() {
        let up = Label::new(Some(uploader));
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
    let q_label = Label::new(Some("Quality"));
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
        let radio = match &first_btn {
            Some(first) => {
                let r = CheckButton::new();
                r.set_group(Some(first));
                r
            }
            None => CheckButton::new(),
        };
        if first_btn.is_none() {
            first_btn = Some(radio.clone());
            radio.set_active(true);
        }

        let label = Label::new(None);
        label.set_markup(&format!(
            "<b>{}</b>  <span color='#585b70'>{}</span>",
            q.label, q.desc
        ));
        label.set_halign(gtk4::Align::Start);

        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.append(&radio);
        row.append(&label);

        let sel = selected.clone();
        let qid = q.id.to_string();
        radio.connect_toggled(move |btn| {
            if btn.is_active() {
                *sel.borrow_mut() = qid.clone();
            }
        });

        quality_box.append(&row);
    }

    scroll.set_child(Some(&quality_box));
    content.append(&scroll);

    // Buttons
    let btn_box = GtkBox::new(Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);

    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("btn-action");
    cancel_btn.add_css_class("btn-cancel");

    let dl_btn = Button::with_label("Download");
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

    dialog.show();

    // GTK4 uses async dialogs differently — blocking approach via nested main loop
    let sel_result = selected.clone();
    let main_context = glib::MainContext::default();
    let response_val = std::rc::Rc::new(std::cell::RefCell::new(ResponseType::Cancel));
    let responded_flg = std::rc::Rc::new(std::cell::RefCell::new(false));

    let rv = response_val.clone();
    let rf = responded_flg.clone();
    dialog.connect_response(move |d, resp| {
        *rv.borrow_mut() = resp;
        *rf.borrow_mut() = true;
        d.close();
    });

    // Kalau user menutup lewat tombol close window (bukan tombol dialog),
    // GTK4 tidak selalu emit response → loop bisa menggantung. Beri nilai
    // Cancel kalau belum ada response, supaya dialog pasti selesai.
    let rv2 = response_val.clone();
    let rf2 = responded_flg.clone();
    dialog.connect_close_request(move |_| {
        if !*rf2.borrow() {
            *rv2.borrow_mut() = ResponseType::Cancel;
        }
        glib::Propagation::Proceed
    });

    // Block until dialog closes
    while dialog.is_visible() {
        main_context.iteration(true);
    }

    if *response_val.borrow() == ResponseType::Ok {
        Some(sel_result.borrow().clone())
    } else {
        None
    }
}
