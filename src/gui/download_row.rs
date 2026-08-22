use crate::downloader::types::{DownloadInfo, DownloadStatus};
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Label, Orientation, ProgressBar,
};

pub struct DownloadRow {
    pub root: gtk4::ListBoxRow,

    filename_lbl: Label,
    status_lbl: Label,
    progress_bar: ProgressBar,
    pct_label: Label,
    size_lbl: Label,
    speed_lbl: Label,
    eta_lbl: Label,
    error_lbl: Label,
    error_box: GtkBox,

    pub pause_btn: Button,
    pub resume_btn: Button,
    pub retry_btn: Button,
    pub cancel_btn: Button,
    pub open_btn: Button,
    pub remove_btn: Button,
}

impl DownloadRow {
    pub fn new(info: &DownloadInfo) -> Self {
        let root = gtk4::ListBoxRow::new();
        root.set_selectable(false);
        root.set_activatable(false);
        root.add_css_class("download-card");

        let outer = GtkBox::new(Orientation::Vertical, 6);
        outer.add_css_class("card-inner");

        // Row 1: filename + badge
        let row1 = GtkBox::new(Orientation::Horizontal, 10);

        let filename_lbl = Label::new(Some(&info.filename));
        filename_lbl.set_hexpand(true);
        filename_lbl.set_halign(gtk4::Align::Start);
        filename_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        filename_lbl.add_css_class("filename-label");

        let status_lbl = Label::new(Some(&info.status.to_string().to_uppercase()));
        status_lbl.add_css_class("badge");

        row1.append(&filename_lbl);
        row1.append(&status_lbl);

        // Row 2: progress + pct
        let row2 = GtkBox::new(Orientation::Horizontal, 10);

        let progress_bar = ProgressBar::new();
        progress_bar.set_hexpand(true);
        progress_bar.set_fraction(info.progress / 100.0);

        let pct_label = Label::new(Some(&format!("{:.1}%", info.progress)));
        pct_label.add_css_class("progress-text");
        pct_label.set_width_chars(6);
        pct_label.set_halign(gtk4::Align::End);

        row2.append(&progress_bar);
        row2.append(&pct_label);

        // Row 3: details
        let row3 = GtkBox::new(Orientation::Horizontal, 0);

        let size_lbl = Label::new(Some(&format!(
            "{} / {}", info.downloaded_fmt(), info.total_size_fmt()
        )));
        size_lbl.set_hexpand(true);
        size_lbl.set_halign(gtk4::Align::Start);
        size_lbl.add_css_class("detail-label");

        let speed_lbl = Label::new(Some(&info.speed_fmt()));
        speed_lbl.add_css_class("detail-speed");
        speed_lbl.set_halign(gtk4::Align::End);

        let eta_lbl = Label::new(Some(&info.eta_fmt()));
        eta_lbl.add_css_class("detail-label");
        eta_lbl.set_halign(gtk4::Align::End);
        eta_lbl.set_margin_start(16);

        row3.append(&size_lbl);
        row3.append(&speed_lbl);
        row3.append(&eta_lbl);

        // Row 3b: error
        let error_box = GtkBox::new(Orientation::Horizontal, 6);
        error_box.set_visible(false);

        let error_icon = Label::new(Some("\u{25CF}")); // ●
        error_icon.add_css_class("error-label");

        let error_lbl = Label::new(None::<&str>);
        error_lbl.set_hexpand(true);
        error_lbl.set_halign(gtk4::Align::Start);
        error_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        error_lbl.add_css_class("error-label");

        error_box.append(&error_icon);
        error_box.append(&error_lbl);

        // Row 4: buttons
        let row4 = GtkBox::new(Orientation::Horizontal, 6);
        row4.set_halign(gtk4::Align::End);
        row4.set_margin_top(4);

        let pause_btn  = make_btn("Pause",       &["btn-action", "btn-pause"]);
        let resume_btn = make_btn("Resume",      &["btn-action", "btn-resume"]);
        let retry_btn  = make_btn("Retry",       &["btn-action", "btn-retry"]);
        let cancel_btn = make_btn("Cancel",      &["btn-action", "btn-cancel"]);
        let open_btn   = make_btn("Open Folder", &["btn-action", "btn-open"]);
        let remove_btn = make_btn("Remove",      &["btn-action", "btn-remove"]);

        row4.append(&retry_btn);
        row4.append(&resume_btn);
        row4.append(&pause_btn);
        row4.append(&cancel_btn);
        row4.append(&open_btn);
        row4.append(&remove_btn);

        // Assemble
        outer.append(&row1);
        outer.append(&row2);
        outer.append(&row3);
        outer.append(&error_box);
        outer.append(&row4);
        root.set_child(Some(&outer));

        let mut row = Self {
            root,
            filename_lbl,
            status_lbl,
            progress_bar,
            pct_label,
            size_lbl,
            speed_lbl,
            eta_lbl,
            error_lbl,
            error_box,
            pause_btn,
            resume_btn,
            retry_btn,
            cancel_btn,
            open_btn,
            remove_btn,
        };
        row.update_buttons(&info.status);
        row.update_badge(&info.status);
        row
    }

    pub fn update(&mut self, info: &DownloadInfo) {
        self.filename_lbl.set_text(&info.filename);
        self.progress_bar.set_fraction((info.progress / 100.0).min(1.0));
        self.pct_label.set_text(&format!("{:.1}%", info.progress));
        self.size_lbl.set_text(&format!(
            "{} / {}", info.downloaded_fmt(), info.total_size_fmt()
        ));

        if info.speed > 0 {
            self.speed_lbl.set_text(&info.speed_fmt());
        } else {
            self.speed_lbl.set_text("");
        }

        if info.eta > 0 && matches!(info.status, DownloadStatus::Downloading) {
            self.eta_lbl.set_text(&info.eta_fmt());
        } else {
            self.eta_lbl.set_text("");
        }

        self.status_lbl.set_text(&info.status.to_string().to_uppercase());
        self.update_badge(&info.status);
        self.update_progress_class(&info.status);

        // Error
        if !info.error_msg.is_empty() {
            self.error_lbl.set_text(&info.error_msg);
            self.error_box.set_visible(true);
        } else {
            self.error_box.set_visible(false);
        }

        self.update_buttons(&info.status);
    }

    fn update_badge(&self, status: &DownloadStatus) {
        let classes = [
            "badge-downloading", "badge-resolving", "badge-completed",
            "badge-error", "badge-paused", "badge-cancelled",
        ];
        for c in &classes {
            self.status_lbl.remove_css_class(c);
        }

        let cls = match status {
            DownloadStatus::Downloading => "badge-downloading",
            DownloadStatus::Resolving   => "badge-resolving",
            DownloadStatus::Completed   => "badge-completed",
            DownloadStatus::Error       => "badge-error",
            DownloadStatus::Paused      => "badge-paused",
            DownloadStatus::Cancelled   => "badge-cancelled",
            DownloadStatus::Queued      => "badge-paused",
        };
        self.status_lbl.add_css_class(cls);
    }

    fn update_progress_class(&self, status: &DownloadStatus) {
        for c in &["completed", "error", "paused"] {
            self.progress_bar.remove_css_class(c);
        }
        match status {
            DownloadStatus::Completed => self.progress_bar.add_css_class("completed"),
            DownloadStatus::Error     => self.progress_bar.add_css_class("error"),
            DownloadStatus::Paused    => self.progress_bar.add_css_class("paused"),
            _ => {}
        }
    }

    fn update_buttons(&mut self, status: &DownloadStatus) {
        let active = matches!(status, DownloadStatus::Downloading | DownloadStatus::Resolving);
        let paused = matches!(status, DownloadStatus::Paused);
        let error  = matches!(status, DownloadStatus::Error);
        let done   = matches!(status, DownloadStatus::Completed);
        let cancel = matches!(status, DownloadStatus::Cancelled);
        let queued = matches!(status, DownloadStatus::Queued);

        self.pause_btn.set_visible(active);
        self.resume_btn.set_visible(paused);
        self.retry_btn.set_visible(error);
        self.cancel_btn.set_visible(active || paused || error || queued);
        self.open_btn.set_visible(done);
        self.remove_btn.set_visible(done || cancel || error);
    }
}

fn make_btn(label: &str, classes: &[&str]) -> Button {
    let btn = Button::with_label(label);
    for c in classes {
        btn.add_css_class(c);
    }
    btn
}
