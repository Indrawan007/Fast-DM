# gui/manager.py

import os
import subprocess
import threading
import time
import re
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib, Gdk, Pango

from engine import DownloadEngine, DownloadStatus, Config
from engine.utils import format_size, format_speed, format_eta
from engine.youtube import (
    is_youtube_url, is_playlist_url, check_ytdlp,
    get_video_info, get_all_browsers, export_cookies,
    YouTubeDownloader, QUALITY_PRESETS, QUALITY_ORDER,
)

# ══════════════════════════════════════════════════════════
# CSS Theme — Modern Glassmorphism + Catppuccin Mocha
# ══════════════════════════════════════════════════════════

CSS = """

/* ── Window ── */
window {
    background-color: #11111b;
}

/* ── Header Bar ── */
.header-box {
    background: linear-gradient(135deg, #1e1e2e 0%, #181825 100%);
    padding: 14px 20px;
    border-bottom: 1px solid rgba(137, 180, 250, 0.15);
}
.header-title {
    color: #cdd6f4;
    font-size: 16px;
    font-weight: 800;
    letter-spacing: 0.5px;
}
.header-subtitle {
    color: #585b70;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1px;
}

/* ── Toolbar ── */
.toolbar-box {
    background-color: #181825;
    padding: 10px 16px;
    border-bottom: 1px solid rgba(69, 71, 90, 0.5);
}
.url-entry {
    background-color: #1e1e2e;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 13px;
    transition: all 200ms ease;
}
.url-entry:focus {
    border-color: #89b4fa;
    box-shadow: 0 0 0 2px rgba(137, 180, 250, 0.2);
}

/* ── Buttons ── */
.btn-download {
    padding: 10px 20px;
    background: linear-gradient(135deg, #89b4fa 0%, #74c7ec 100%);
    color: #11111b;
    border: none;
    border-radius: 10px;
    font-weight: 800;
    font-size: 13px;
    letter-spacing: 0.3px;
    transition: all 200ms ease;
}
.btn-download:hover {
    background: linear-gradient(135deg, #b4d0fb 0%, #89dceb 100%);
}
.btn-download:active {
    background: linear-gradient(135deg, #74a8f7 0%, #5cb8d6 100%);
}
.btn-clear {
    padding: 10px 16px;
    background-color: transparent;
    color: #6c7086;
    border: 1px solid #313244;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 600;
    transition: all 200ms ease;
}
.btn-clear:hover {
    background-color: #1e1e2e;
    color: #a6adc8;
    border-color: #45475a;
}

/* ── Download List ── */
.download-list {
    background-color: #11111b;
}

/* ── Download Card ── */
.download-card {
    background-color: #1e1e2e;
    border-radius: 12px;
    border: 1px solid #313244;
    margin: 4px 12px;
    padding: 0;
    transition: all 200ms ease;
}
.download-card:hover {
    background-color: #232336;
    border-color: #45475a;
}

.card-inner {
    padding: 14px 16px;
}

/* ── Filename ── */
.filename-label {
    color: #cdd6f4;
    font-weight: 700;
    font-size: 13px;
}

/* ── Status Badge ── */
.badge {
    font-size: 9px;
    font-weight: 800;
    padding: 3px 10px;
    border-radius: 20px;
    letter-spacing: 0.8px;
}
.badge-downloading {
    background-color: rgba(137, 180, 250, 0.15);
    color: #89b4fa;
}
.badge-resolving {
    background-color: rgba(116, 199, 236, 0.15);
    color: #74c7ec;
}
.badge-completed {
    background-color: rgba(166, 227, 161, 0.15);
    color: #a6e3a1;
}
.badge-error {
    background-color: rgba(243, 139, 168, 0.15);
    color: #f38ba8;
}
.badge-paused {
    background-color: rgba(250, 179, 135, 0.15);
    color: #fab387;
}
.badge-cancelled {
    background-color: rgba(108, 112, 134, 0.15);
    color: #6c7086;
}

/* ── Progress Bar ── */
.progress-trough {
    min-height: 4px;
    border-radius: 2px;
    background-color: #313244;
}
progressbar trough {
    min-height: 4px;
    border-radius: 2px;
    background-color: #313244;
}
progressbar progress {
    min-height: 4px;
    border-radius: 2px;
    background-image: linear-gradient(90deg, #89b4fa, #74c7ec);
    transition: all 300ms ease;
}
progressbar.completed progress {
    background-image: linear-gradient(90deg, #a6e3a1, #94e2d5);
}
progressbar.error progress {
    background-image: linear-gradient(90deg, #f38ba8, #eba0ac);
}
progressbar.paused progress {
    background-image: linear-gradient(90deg, #fab387, #f9e2af);
}

/* ── Detail Labels ── */
.detail-label {
    color: #585b70;
    font-size: 11px;
    font-weight: 500;
}
.detail-speed {
    color: #89b4fa;
    font-size: 11px;
    font-weight: 700;
}
.progress-text {
    color: #a6adc8;
    font-size: 11px;
    font-weight: 700;
}

/* ── Error Message ── */
.error-label {
    color: #f38ba8;
    font-size: 11px;
    font-weight: 500;
    font-style: italic;
}
.retry-label {
    color: #fab387;
    font-size: 11px;
    font-weight: 500;
}
.error-icon {
    color: #f38ba8;
    font-size: 12px;
}

/* ── Action Buttons ── */
.btn-action {
    padding: 5px 14px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: 700;
    border: none;
    transition: all 150ms ease;
    letter-spacing: 0.3px;
}

.btn-pause {
    background-color: rgba(250, 179, 135, 0.12);
    color: #fab387;
    border: 1px solid rgba(250, 179, 135, 0.25);
}
.btn-pause:hover {
    background-color: rgba(250, 179, 135, 0.25);
}

.btn-resume {
    background-color: rgba(166, 227, 161, 0.12);
    color: #a6e3a1;
    border: 1px solid rgba(166, 227, 161, 0.25);
}
.btn-resume:hover {
    background-color: rgba(166, 227, 161, 0.25);
}

.btn-retry {
    background-color: rgba(137, 180, 250, 0.12);
    color: #89b4fa;
    border: 1px solid rgba(137, 180, 250, 0.25);
}
.btn-retry:hover {
    background-color: rgba(137, 180, 250, 0.25);
}

.btn-cancel {
    background-color: rgba(243, 139, 168, 0.08);
    color: #f38ba8;
    border: 1px solid rgba(243, 139, 168, 0.2);
}
.btn-cancel:hover {
    background-color: rgba(243, 139, 168, 0.2);
}

.btn-open {
    background: linear-gradient(135deg, #89b4fa 0%, #74c7ec 100%);
    color: #11111b;
    border: none;
}
.btn-open:hover {
    background: linear-gradient(135deg, #b4d0fb 0%, #89dceb 100%);
}

.btn-remove {
    background-color: transparent;
    color: #585b70;
    border: 1px solid #313244;
}
.btn-remove:hover {
    background-color: #1e1e2e;
    color: #6c7086;
}

/* ── Stats Bar ── */
.stats-box {
    background-color: #181825;
    padding: 8px 20px;
    border-top: 1px solid rgba(69, 71, 90, 0.5);
}
.stats-label {
    color: #45475a;
    font-size: 11px;
    font-weight: 600;
}
.stats-value {
    color: #6c7086;
    font-size: 11px;
    font-weight: 700;
}
.stats-speed-value {
    color: #89b4fa;
    font-size: 11px;
    font-weight: 800;
}

/* ── Placeholder ── */
.placeholder-icon {
    color: #313244;
    font-size: 48px;
}
.placeholder-title {
    color: #45475a;
    font-size: 15px;
    font-weight: 700;
}
.placeholder-subtitle {
    color: #313244;
    font-size: 12px;
}

/* ── Scrollbar ── */
scrolledwindow scrollbar {
    background-color: transparent;
}
scrolledwindow scrollbar slider {
    background-color: #313244;
    border-radius: 10px;
    min-width: 4px;
    min-height: 20px;
}
scrolledwindow scrollbar slider:hover {
    background-color: #45475a;
}
"""


# ══════════════════════════════════════════════════════════
# Download Row Widget
# ══════════════════════════════════════════════════════════

class DownloadRow(Gtk.ListBoxRow):

    def __init__(self, dl_data):
        super().__init__()
        self.dl_id = dl_data["id"]
        self.get_style_context().add_class("download-card")

        # No selection highlight
        self.set_selectable(False)
        self.set_activatable(False)

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        outer.get_style_context().add_class("card-inner")

        # ── Row 1: Filename + Badge ──
        row1 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)

        self.filename_lbl = Gtk.Label(xalign=0.0)
        self.filename_lbl.set_markup(
            "<b>{}</b>".format(GLib.markup_escape_text(dl_data["filename"]))
        )
        self.filename_lbl.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        self.filename_lbl.set_hexpand(True)
        self.filename_lbl.get_style_context().add_class("filename-label")

        self.status_lbl = Gtk.Label(label=dl_data["status"].upper())
        self.status_lbl.get_style_context().add_class("badge")
        self._update_badge(dl_data["status"])

        row1.pack_start(self.filename_lbl, True, True, 0)
        row1.pack_end(self.status_lbl, False, False, 0)

        # ── Row 2: Progress bar + percentage ──
        row2 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)

        self.progress_bar = Gtk.ProgressBar()
        self.progress_bar.set_hexpand(True)
        self.progress_bar.set_show_text(False)
        self._set_progress(dl_data)

        self.pct_label = Gtk.Label()
        self.pct_label.get_style_context().add_class("progress-text")
        self.pct_label.set_text("{:.1f}%".format(dl_data["progress"]))
        self.pct_label.set_width_chars(6)
        self.pct_label.set_xalign(1.0)

        row2.pack_start(self.progress_bar, True, True, 0)
        row2.pack_end(self.pct_label, False, False, 0)

        # ── Row 3: Details ──
        row3 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)

        self.size_lbl = Gtk.Label(xalign=0.0)
        self.size_lbl.get_style_context().add_class("detail-label")
        self.size_lbl.set_hexpand(True)

        self.speed_lbl = Gtk.Label(xalign=1.0)
        self.speed_lbl.get_style_context().add_class("detail-speed")

        self.eta_lbl = Gtk.Label(xalign=1.0)
        self.eta_lbl.get_style_context().add_class("detail-label")
        self.eta_lbl.set_margin_start(16)

        self._set_details(dl_data)

        row3.pack_start(self.size_lbl, True, True, 0)
        row3.pack_end(self.eta_lbl, False, False, 0)
        row3.pack_end(self.speed_lbl, False, False, 0)

        # ── Row 3b: Error ──
        self.error_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        self.error_icon = Gtk.Label(label="●")
        self.error_icon.get_style_context().add_class("error-icon")
        self.error_lbl = Gtk.Label(xalign=0.0)
        self.error_lbl.set_ellipsize(Pango.EllipsizeMode.END)
        self.error_lbl.set_hexpand(True)
        self.error_lbl.get_style_context().add_class("error-label")
        self.error_box.pack_start(self.error_icon, False, False, 0)
        self.error_box.pack_start(self.error_lbl, True, True, 0)
        self.error_box.set_no_show_all(True)
        self.error_box.hide()

        # ── Row 4: Buttons ──
        row4 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        row4.set_halign(Gtk.Align.END)
        row4.set_margin_top(4)

        self.pause_btn  = self._make_btn("Pause",   "btn-pause")
        self.resume_btn = self._make_btn("Resume",  "btn-resume")
        self.retry_btn  = self._make_btn("Retry",   "btn-retry")
        self.cancel_btn = self._make_btn("Cancel",  "btn-cancel")
        self.open_btn   = self._make_btn("Open Folder", "btn-open")
        self.remove_btn = self._make_btn("Remove",  "btn-remove")

        for btn in (self.retry_btn, self.resume_btn, self.pause_btn,
                    self.cancel_btn, self.open_btn, self.remove_btn):
            row4.pack_start(btn, False, False, 0)

        # ── Assemble ──
        outer.pack_start(row1, False, False, 0)
        outer.pack_start(row2, False, False, 0)
        outer.pack_start(row3, False, False, 0)
        outer.pack_start(self.error_box, False, False, 0)
        outer.pack_start(row4, False, False, 0)
        self.add(outer)

        self._update_buttons(dl_data["status"])

    def _make_btn(self, label, css_class):
        btn = Gtk.Button(label=label)
        ctx = btn.get_style_context()
        ctx.add_class("btn-action")
        ctx.add_class(css_class)
        return btn

    def _set_progress(self, d):
        pct = float(d["progress"])
        self.progress_bar.set_fraction(min(pct / 100.0, 1.0))

        ctx = self.progress_bar.get_style_context()
        for c in ("completed", "error", "paused"):
            ctx.remove_class(c)
        status = d["status"]
        if status == "completed":
            ctx.add_class("completed")
        elif status == "error":
            ctx.add_class("error")
        elif status == "paused":
            ctx.add_class("paused")

    def _set_details(self, d):
        self.size_lbl.set_text(
            "{} / {}".format(d["downloaded_fmt"], d["total_size_fmt"])
        )
        speed = d["speed"]
        if speed > 0:
            self.speed_lbl.set_text(d["speed_fmt"])
        else:
            self.speed_lbl.set_text("")

        eta = d.get("eta", 0)
        if eta > 0 and d["status"] == "downloading":
            self.eta_lbl.set_text("{}".format(d["eta_fmt"]))
        else:
            self.eta_lbl.set_text("")

    def _update_badge(self, status):
        ctx = self.status_lbl.get_style_context()
        for c in ("badge-downloading", "badge-completed", "badge-error",
                   "badge-paused", "badge-cancelled", "badge-resolving"):
            ctx.remove_class(c)

        badge_map = {
            "downloading": "badge-downloading",
            "completed":   "badge-completed",
            "error":       "badge-error",
            "paused":      "badge-paused",
            "cancelled":   "badge-cancelled",
            "resolving":   "badge-resolving",
            "queued":      "badge-paused",
        }
        cls = badge_map.get(status)
        if cls:
            ctx.add_class(cls)

    def _update_buttons(self, status):
        active    = status in ("downloading", "resolving")
        paused    = status == "paused"
        error     = status == "error"
        done      = status == "completed"
        cancelled = status == "cancelled"

        self.pause_btn.set_visible(active)
        self.resume_btn.set_visible(paused)
        self.retry_btn.set_visible(error)
        self.cancel_btn.set_visible(active or paused or error)
        self.open_btn.set_visible(done)
        self.remove_btn.set_visible(done or cancelled or error)

    def update(self, d):
        self._set_progress(d)
        self._set_details(d)

        self.pct_label.set_text("{:.1f}%".format(d["progress"]))

        self.filename_lbl.set_markup(
            "<b>{}</b>".format(GLib.markup_escape_text(d["filename"]))
        )

        status = d["status"]
        self.status_lbl.set_text(status.upper())
        self._update_badge(status)

        # Error message
        err = d.get("error_msg", "")
        if err and status == "error":
            self.error_lbl.set_text(err)
            self.error_lbl.get_style_context().remove_class("retry-label")
            self.error_lbl.get_style_context().add_class("error-label")
            self.error_box.set_no_show_all(False)
            self.error_box.show_all()
        elif err and status == "downloading":
            self.error_lbl.set_text(err)
            self.error_lbl.get_style_context().remove_class("error-label")
            self.error_lbl.get_style_context().add_class("retry-label")
            self.error_box.set_no_show_all(False)
            self.error_box.show_all()
        else:
            self.error_box.hide()

        self._update_buttons(status)

# ==========================================================
# Youtube
# ==========================================================

class YouTubeDialog(Gtk.Dialog):
    """Dialog pilih kualitas YouTube — mirip IDM."""

    def __init__(self, parent, video_info):
        super().__init__(
            title="YouTube Download",
            transient_for=parent,
            modal=True,
        )
        self.set_default_size(500, 520)
        self.set_resizable(False)
        self.video_info = video_info
        self.selected_quality = "best_mp4"
        self.selected_subtitle = None

        content = self.get_content_area()
        content.set_spacing(10)
        content.set_margin_top(16)
        content.set_margin_bottom(12)
        content.set_margin_start(20)
        content.set_margin_end(20)

        # ── Video Title ──
        title_lbl = Gtk.Label(xalign=0.0)
        title_text = GLib.markup_escape_text(video_info["title"])
        title_lbl.set_markup("<b>{}</b>".format(title_text))
        title_lbl.set_line_wrap(True)
        title_lbl.set_max_width_chars(55)
        title_lbl.get_style_context().add_class("filename-label")
        content.pack_start(title_lbl, False, False, 0)

        # ── Info Row ──
        info_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)

        if video_info.get("uploader"):
            up_lbl = Gtk.Label(
                label="{}".format(video_info["uploader"])
            )
            up_lbl.get_style_context().add_class("detail-label")
            info_box.pack_start(up_lbl, False, False, 0)

        if video_info.get("duration_str"):
            dur_lbl = Gtk.Label(
                label="{}".format(video_info["duration_str"])
            )
            dur_lbl.get_style_context().add_class("detail-label")
            info_box.pack_start(dur_lbl, False, False, 0)

        views = video_info.get("view_count", 0)
        if views:
            if views >= 1_000_000:
                view_str = "{:.1f}M views".format(views / 1_000_000)
            elif views >= 1_000:
                view_str = "{:.0f}K views".format(views / 1_000)
            else:
                view_str = "{} views".format(views)
            view_lbl = Gtk.Label(label=view_str)
            view_lbl.get_style_context().add_class("detail-label")
            info_box.pack_start(view_lbl, False, False, 0)

        content.pack_start(info_box, False, False, 0)

        # ── Separator ──
        content.pack_start(Gtk.Separator(), False, False, 4)

        # ── Quality Selection ──
        q_label = Gtk.Label(xalign=0.0)
        q_label.set_markup("<b>Quality</b>")
        q_label.get_style_context().add_class("filename-label")
        content.pack_start(q_label, False, False, 0)

        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        scroll.set_min_content_height(220)

        quality_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        first_btn = None

        for key in QUALITY_ORDER:
            preset = QUALITY_PRESETS.get(key)
            if not preset:
                continue

            radio = Gtk.RadioButton.new_from_widget(first_btn)
            if first_btn is None:
                first_btn = radio

            label_box = Gtk.Box(
                orientation=Gtk.Orientation.HORIZONTAL, spacing=8
            )

            name_lbl = Gtk.Label(xalign=0.0)
            name_lbl.set_markup("<b>{}</b>".format(preset["label"]))
            name_lbl.get_style_context().add_class("filename-label")

            desc_lbl = Gtk.Label(xalign=0.0)
            desc_lbl.set_text(preset["desc"])
            desc_lbl.get_style_context().add_class("detail-label")

            label_box.pack_start(name_lbl, False, False, 0)
            label_box.pack_start(desc_lbl, False, False, 0)

            radio.add(label_box)
            radio.connect("toggled", self._on_quality_toggled, key)
            quality_box.pack_start(radio, False, False, 0)

        scroll.add(quality_box)
        content.pack_start(scroll, True, True, 0)

        # ── Subtitle Checkbox ──
        subs = video_info.get("available_subs", [])
        if subs:
            content.pack_start(Gtk.Separator(), False, False, 4)

            sub_box = Gtk.Box(
                orientation=Gtk.Orientation.HORIZONTAL, spacing=8
            )

            self.sub_check = Gtk.CheckButton(label="Download subtitle:")
            self.sub_check.get_style_context().add_class("detail-label")

            self.sub_combo = Gtk.ComboBoxText()
            for lang in subs:
                self.sub_combo.append_text(lang)
            if "en" in subs:
                idx = subs.index("en")
                self.sub_combo.set_active(idx)
            elif subs:
                self.sub_combo.set_active(0)

            self.sub_combo.set_sensitive(False)
            self.sub_check.connect("toggled", self._on_sub_toggled)

            sub_box.pack_start(self.sub_check, False, False, 0)
            sub_box.pack_start(self.sub_combo, False, False, 0)
            content.pack_start(sub_box, False, False, 0)

        # ── Buttons ──
        btn_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        btn_box.set_halign(Gtk.Align.END)
        btn_box.set_margin_top(8)

        cancel_btn = Gtk.Button(label="Cancel")
        cancel_btn.get_style_context().add_class("btn-action")
        cancel_btn.get_style_context().add_class("btn-cancel")
        cancel_btn.connect("clicked", lambda _: self.response(
            Gtk.ResponseType.CANCEL
        ))

        dl_btn = Gtk.Button(label="Download")
        dl_btn.get_style_context().add_class("btn-download")
        dl_btn.connect("clicked", lambda _: self.response(
            Gtk.ResponseType.OK
        ))

        btn_box.pack_start(cancel_btn, False, False, 0)
        btn_box.pack_start(dl_btn, False, False, 0)
        content.pack_start(btn_box, False, False, 0)

        self.show_all()

    def _on_quality_toggled(self, button, quality):
        if button.get_active():
            self.selected_quality = quality

    def _on_sub_toggled(self, button):
        self.sub_combo.set_sensitive(button.get_active())
        if button.get_active():
            self.selected_subtitle = self.sub_combo.get_active_text()
        else:
            self.selected_subtitle = None


class YouTubePlaylistDialog(Gtk.Dialog):
    """Dialog untuk download playlist YouTube."""

    def __init__(self, parent, playlist_info):
        super().__init__(
            title="YouTube Playlist",
            transient_for=parent,
            modal=True,
        )
        self.set_default_size(500, 450)
        self.playlist_info = playlist_info
        self.selected_quality = "best_mp4"
        self.selected_entries = list(range(len(
            playlist_info.get("entries", [])
        )))

        content = self.get_content_area()
        content.set_spacing(10)
        content.set_margin_top(16)
        content.set_margin_bottom(12)
        content.set_margin_start(20)
        content.set_margin_end(20)

        # Title
        title_lbl = Gtk.Label(xalign=0.0)
        title_lbl.set_markup(
            "<b>{}</b>".format(
                GLib.markup_escape_text(playlist_info["title"])
            )
        )
        title_lbl.get_style_context().add_class("filename-label")
        content.pack_start(title_lbl, False, False, 0)

        count_lbl = Gtk.Label(
            label="{} videos".format(playlist_info["count"]),
            xalign=0.0
        )
        count_lbl.get_style_context().add_class("detail-label")
        content.pack_start(count_lbl, False, False, 0)

        content.pack_start(Gtk.Separator(), False, False, 4)

        # Quality dropdown
        q_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        q_lbl = Gtk.Label(label="Quality:")
        q_lbl.get_style_context().add_class("filename-label")

        self.q_combo = Gtk.ComboBoxText()
        for key in QUALITY_ORDER:
            preset = QUALITY_PRESETS.get(key)
            if preset:
                self.q_combo.append(key, preset["label"])
        self.q_combo.set_active_id("best_mp4")
        self.q_combo.connect("changed", self._on_q_changed)

        q_box.pack_start(q_lbl, False, False, 0)
        q_box.pack_start(self.q_combo, True, True, 0)
        content.pack_start(q_box, False, False, 0)

        # Video list
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)

        listbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)

        entries = playlist_info.get("entries", [])
        self.checks = []

        for i, entry in enumerate(entries):
            check = Gtk.CheckButton()
            check.set_active(True)
            label = Gtk.Label(xalign=0.0)
            label.set_markup(
                "{}. <b>{}</b>".format(
                    i + 1, GLib.markup_escape_text(entry["title"])
                )
            )
            label.set_ellipsize(Pango.EllipsizeMode.END)

            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
            row.pack_start(check, False, False, 0)
            row.pack_start(label, True, True, 0)

            listbox.pack_start(row, False, False, 0)
            self.checks.append(check)

        scroll.add(listbox)
        content.pack_start(scroll, True, True, 0)

        # Buttons
        btn_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        btn_box.set_halign(Gtk.Align.END)
        btn_box.set_margin_top(8)

        cancel_btn = Gtk.Button(label="Cancel")
        cancel_btn.get_style_context().add_class("btn-action")
        cancel_btn.get_style_context().add_class("btn-cancel")
        cancel_btn.connect("clicked", lambda _: self.response(
            Gtk.ResponseType.CANCEL
        ))

        dl_btn = Gtk.Button(
            label="Download {} videos".format(len(entries))
        )
        dl_btn.get_style_context().add_class("btn-download")
        dl_btn.connect("clicked", lambda _: self.response(
            Gtk.ResponseType.OK
        ))

        btn_box.pack_start(cancel_btn, False, False, 0)
        btn_box.pack_start(dl_btn, False, False, 0)
        content.pack_start(btn_box, False, False, 0)

        self.show_all()

    def _on_q_changed(self, combo):
        self.selected_quality = combo.get_active_id() or "best_mp4"

    def get_selected_entries(self):
        selected = []
        entries = self.playlist_info.get("entries", [])
        for i, check in enumerate(self.checks):
            if check.get_active() and i < len(entries):
                selected.append(entries[i])
        return selected

# ══════════════════════════════════════════════════════════
# Main Window
# ══════════════════════════════════════════════════════════

class ManagerWindow(Gtk.Window):

    def __init__(self, engine):
        super().__init__(title="Fast Download Manager")
        self.engine = engine
        self._rows = {}

        self.set_default_size(780, 580)
        self.set_position(Gtk.WindowPosition.CENTER)
        self.connect("delete-event", self._on_quit)

        # Window icon
        try:
            script_dir = os.path.dirname(os.path.dirname(
                os.path.abspath(__file__)))
            for icon_name in ("fast-dm-icon.png",
                              "extension/icons/icon128.png"):
                icon_path = os.path.join(script_dir, icon_name)
                if os.path.exists(icon_path):
                    self.set_icon_from_file(icon_path)
                    break
        except Exception:
            pass

        # CSS
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS.encode("utf-8"))
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(),
            provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

        self._build_ui()

        engine.set_callbacks(
            on_update=self._cb_update,
            on_complete=self._cb_complete,
        )

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)

        # ── Header ──
        header = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        header.get_style_context().add_class("header-box")

        title_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)

        title = Gtk.Label(label="⚡ Fast Download Manager")
        title.get_style_context().add_class("header-title")
        title.set_halign(Gtk.Align.START)

        subtitle = Gtk.Label(label="POWERED BY ARIA2")
        subtitle.get_style_context().add_class("header-subtitle")
        subtitle.set_halign(Gtk.Align.START)
        subtitle.set_valign(Gtk.Align.END)

        title_row.pack_start(title, False, False, 0)
        title_row.pack_start(subtitle, False, False, 4)

        header.pack_start(title_row, False, False, 0)
        root.pack_start(header, False, False, 0)

        # ── Toolbar ──
        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        toolbar.get_style_context().add_class("toolbar-box")

        self.url_entry = Gtk.Entry()
        self.url_entry.set_placeholder_text("Paste download URL here…")
        self.url_entry.set_hexpand(True)
        self.url_entry.get_style_context().add_class("url-entry")
        self.url_entry.connect("activate", self._on_add_url)

        add_btn = Gtk.Button(label="Download")
        add_btn.get_style_context().add_class("btn-download")
        add_btn.connect("clicked", self._on_add_url)

        clear_btn = Gtk.Button(label="Clear Done")
        clear_btn.get_style_context().add_class("btn-clear")
        clear_btn.connect("clicked", self._on_clear_done)

        toolbar.pack_start(self.url_entry, True, True, 0)
        toolbar.pack_start(add_btn, False, False, 0)
        toolbar.pack_start(clear_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        # ── Download List ──
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)

        self.listbox = Gtk.ListBox()
        self.listbox.set_selection_mode(Gtk.SelectionMode.NONE)
        self.listbox.get_style_context().add_class("download-list")

        # Placeholder
        placeholder = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        placeholder.set_valign(Gtk.Align.CENTER)
        placeholder.set_margin_top(60)
        placeholder.set_margin_bottom(60)

        ph_icon = Gtk.Label(label="⚡")
        ph_icon.get_style_context().add_class("placeholder-icon")

        ph_title = Gtk.Label(label="No downloads yet")
        ph_title.get_style_context().add_class("placeholder-title")

        ph_sub = Gtk.Label(
            label="Paste a URL above or use the browser extension"
        )
        ph_sub.get_style_context().add_class("placeholder-subtitle")

        placeholder.pack_start(ph_icon, False, False, 0)
        placeholder.pack_start(ph_title, False, False, 0)
        placeholder.pack_start(ph_sub, False, False, 0)
        placeholder.show_all()

        self.listbox.set_placeholder(placeholder)

        scroll.add(self.listbox)
        root.pack_start(scroll, True, True, 0)

        # ── Stats Bar ──
        statsbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=24)
        statsbar.get_style_context().add_class("stats-box")

        # Active
        stat1 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
        lbl_a = Gtk.Label(label="Active")
        lbl_a.get_style_context().add_class("stats-label")
        self.lbl_active = Gtk.Label(label="0")
        self.lbl_active.get_style_context().add_class("stats-value")
        stat1.pack_start(lbl_a, False, False, 0)
        stat1.pack_start(self.lbl_active, False, False, 0)

        # Speed
        stat2 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
        lbl_s = Gtk.Label(label="Speed")
        lbl_s.get_style_context().add_class("stats-label")
        self.lbl_speed = Gtk.Label(label="0 B/s")
        self.lbl_speed.get_style_context().add_class("stats-speed-value")
        stat2.pack_start(lbl_s, False, False, 0)
        stat2.pack_start(self.lbl_speed, False, False, 0)

        # Total
        stat3 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
        lbl_t = Gtk.Label(label="Total")
        lbl_t.get_style_context().add_class("stats-label")
        self.lbl_total = Gtk.Label(label="0")
        self.lbl_total.get_style_context().add_class("stats-value")
        stat3.pack_start(lbl_t, False, False, 0)
        stat3.pack_start(self.lbl_total, False, False, 0)

        statsbar.pack_start(stat1, False, False, 0)
        statsbar.pack_start(stat2, False, False, 0)
        statsbar.pack_start(stat3, False, False, 0)
        root.pack_start(statsbar, False, False, 0)

        self.add(root)

    # ──────────────────────────────────────────────────────
    # Download management
    # ──────────────────────────────────────────────────────

    def _on_add_url(self, _w):
        url = self.url_entry.get_text().strip()
        if not url:
            return
        if not url.startswith(("http://", "https://", "ftp://")):
            url = "https://" + url

        # YouTube URL → dialog khusus
        if is_youtube_url(url):
            self.url_entry.set_text("")
            self._handle_youtube(url)
            return

        # Download biasa
        dl_id = self.engine.add_download(url)
        self.url_entry.set_text("")
        self._add_row(dl_id)

    def _handle_youtube(self, url):
        """Handle YouTube URL."""
        if not check_ytdlp():
            dialog = Gtk.MessageDialog(
                transient_for=self, modal=True,
                message_type=Gtk.MessageType.ERROR,
                buttons=Gtk.ButtonsType.OK,
                text="yt-dlp not found",
            )
            dialog.format_secondary_text(
                "Install:\n"
                "  sudo apt install yt-dlp\n"
                "  pip3 install -U yt-dlp"
            )
            dialog.run()
            dialog.destroy()
            return

        self.url_entry.set_placeholder_text("Fetching video info...")
        self.url_entry.set_sensitive(False)

        def _fetch():
            info = get_video_info(url)
            GLib.idle_add(self._show_yt_dialog, url, info)

        threading.Thread(target=_fetch, daemon=True).start()

    def _show_yt_dialog(self, url, info):
        """Tampilkan dialog YouTube setelah info didapat."""
        self.url_entry.set_placeholder_text("Paste download URL here...")
        self.url_entry.set_sensitive(True)

        if not info:
            dialog = Gtk.MessageDialog(
                transient_for=self, modal=True,
                message_type=Gtk.MessageType.ERROR,
                buttons=Gtk.ButtonsType.OK,
                text="Cannot fetch video info",
            )
            dialog.format_secondary_text(
                "The video may be private, age-restricted,\n"
                "or YouTube is blocking the request.\n\n"
                "Try:\n"
                "1. Login YouTube di browser\n"
                "2. Tutup browser\n"
                "3. Coba lagi"
            )
            dialog.run()
            dialog.destroy()
            return

        # Playlist
        if info.get("type") == "playlist":
            dialog = YouTubePlaylistDialog(self, info)
            response = dialog.run()
            quality = dialog.selected_quality
            entries = dialog.get_selected_entries()
            dialog.destroy()

            if response != Gtk.ResponseType.OK or not entries:
                return

            for entry in entries:
                entry_url = entry.get("url", "")
                if entry_url:
                    self._start_yt_download(entry_url, {
                        "title": entry.get("title", "YouTube video"),
                        "type": "video",
                    }, quality, None)
            return

        # Single video
        dialog = YouTubeDialog(self, info)
        response = dialog.run()
        quality = dialog.selected_quality
        subtitle = getattr(dialog, 'selected_subtitle', None)
        dialog.destroy()

        if response != Gtk.ResponseType.OK:
            return

        self._start_yt_download(url, info, quality, subtitle)

    def _start_yt_download(self, url, info, quality, subtitle_lang):
        """Mulai download YouTube video."""
        title = info.get("title", "YouTube video")
        preset = QUALITY_PRESETS.get(quality, QUALITY_PRESETS["best_mp4"])
        ext = preset.get("ext", "mp4")

        dl_id = self.engine.add_download(
            url,
            filename="{}.{}".format(title, ext),
            auto_start=False,
        )
        self._add_row_youtube(dl_id, url, quality, subtitle_lang)

    def _add_row_youtube(self, dl_id, url, quality, subtitle_lang):
        """
        Tambah row untuk YouTube download.
        Tombol Pause/Cancel/Resume langsung di-bind ke yt-dlp.
        Tidak pakai _add_row biasa agar tidak ada handler aria2c.
        """
        dl_data = self.engine.get_download(dl_id)
        if not dl_data:
            return

        row = DownloadRow(dl_data)

        item = self.engine._downloads.get(dl_id)
        if not item:
            return

        from engine.downloader import DownloadStatus
        item.status = DownloadStatus.DOWNLOADING
        GLib.idle_add(self._gtk_update, item.to_dict())

        downloader = YouTubeDownloader(
            save_dir=self.engine.cfg.download_dir
        )
        item._yt_downloader = downloader

        # ── Callbacks ──
        def on_progress(prog):
            if not self.engine._downloads.get(dl_id):
                return
            pct = prog.get("percent", 0)
            if pct >= 0:
                item.progress = pct
            fn = prog.get("filename", "")
            if fn:
                item.filename = os.path.basename(fn)

            speed_str = prog.get("speed", "")
            if speed_str:
                m = re.match(r'([\d.]+)\s*(\S+)', speed_str)
                if m:
                    val = float(m.group(1))
                    unit = m.group(2).lower()
                    if 'gib' in unit:
                        item.speed = int(val * 1073741824)
                    elif 'mib' in unit:
                        item.speed = int(val * 1048576)
                    elif 'kib' in unit:
                        item.speed = int(val * 1024)
                    else:
                        item.speed = int(val)

            status = prog.get("status", "")
            if status == "merging":
                item.error_msg = "Merging video + audio..."
            elif status == "paused":
                item.status = DownloadStatus.PAUSED
                item.speed = 0
            else:
                item.error_msg = ""
                item.status = DownloadStatus.DOWNLOADING

            GLib.idle_add(self._gtk_update, item.to_dict())

        def on_complete(result):
            if not self.engine._downloads.get(dl_id):
                return
            item.status = DownloadStatus.COMPLETED
            item.progress = 100.0
            item.speed = 0
            item.error_msg = ""
            fn = result.get("filename", "")
            if fn:
                item.filename = os.path.basename(fn)
            GLib.idle_add(self._gtk_update, item.to_dict())

        def on_error(err):
            if not self.engine._downloads.get(dl_id):
                return
            item.status = DownloadStatus.ERROR
            item.error_msg = str(err)
            item.speed = 0
            GLib.idle_add(self._gtk_update, item.to_dict())

        # ── Button handlers (YouTube-specific) ──
        def yt_pause(_b):
            downloader.pause()
            item.status = DownloadStatus.PAUSED
            item.speed = 0
            GLib.idle_add(self._gtk_update, item.to_dict())

        def yt_resume(_b):
            item.status = DownloadStatus.DOWNLOADING
            item.error_msg = ""
            GLib.idle_add(self._gtk_update, item.to_dict())
            downloader.resume()

        def yt_cancel(_b):
            downloader.cancel()
            item.status = DownloadStatus.CANCELLED
            item.speed = 0
            GLib.idle_add(self._gtk_update, item.to_dict())

        def yt_retry(_b):
            item.status = DownloadStatus.DOWNLOADING
            item.error_msg = ""
            GLib.idle_add(self._gtk_update, item.to_dict())
            downloader.download(
                url, quality,
                subtitle_lang=subtitle_lang,
                on_progress=on_progress,
                on_complete=on_complete,
                on_error=on_error,
            )

        def on_remove(_b):
            downloader.cancel()
            r = self._rows.pop(dl_id, None)
            if r:
                self.listbox.remove(r)
            self.engine.clear_download(dl_id)

        # Connect buttons langsung ke YouTube handlers
        row.pause_btn.connect("clicked", yt_pause)
        row.resume_btn.connect("clicked", yt_resume)
        row.cancel_btn.connect("clicked", yt_cancel)
        row.retry_btn.connect("clicked", yt_retry)
        row.open_btn.connect(
            "clicked", lambda _b: self._open_folder(dl_data))
        row.remove_btn.connect("clicked", on_remove)

        self._rows[dl_id] = row
        self.listbox.prepend(row)
        self.listbox.show_all()

        # Mulai download
        downloader.download(
            url, quality,
            subtitle_lang=subtitle_lang,
            on_progress=on_progress,
            on_complete=on_complete,
            on_error=on_error,
        )

    def add_download_from_extension(self, url, filename=None, headers=None):
        dl_id = self.engine.add_download(
            url, filename=filename, headers=headers
        )
        GLib.idle_add(self._add_row, dl_id)
        return dl_id

    def _add_row(self, dl_id):
        dl_data = self.engine.get_download(dl_id)
        if not dl_data:
            return

        row = DownloadRow(dl_data)

        row.pause_btn.connect(
            "clicked", lambda _b: self.engine.pause_download(dl_id))
        row.resume_btn.connect(
            "clicked", lambda _b: self.engine.resume_download(dl_id))
        row.retry_btn.connect(
            "clicked", lambda _b: self.engine.retry_download(dl_id))

        def on_cancel(_b):
            self.engine.cancel_download(dl_id)
        row.cancel_btn.connect("clicked", on_cancel)

        row.open_btn.connect(
            "clicked", lambda _b, d=dl_data: self._open_folder(d))

        def on_remove(_b):
            r = self._rows.pop(dl_id, None)
            if r:
                self.listbox.remove(r)
            self.engine.clear_download(dl_id)
        row.remove_btn.connect("clicked", on_remove)

        self._rows[dl_id] = row
        self.listbox.prepend(row)
        self.listbox.show_all()

    def _remove_row(self, dl_id):
        row = self._rows.pop(dl_id, None)
        if row:
            self.listbox.remove(row)
        self.engine.clear_download(dl_id)

    def _open_folder(self, dl_data):
        subprocess.Popen(
            ["xdg-open", dl_data["save_dir"]],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def _on_clear_done(self, _w):
        """Hapus entri selesai. Cancel YouTube downloader jika ada."""
        to_remove = []
        for dl_id in list(self._rows.keys()):
            d = self.engine.get_download(dl_id)
            if d and d["status"] in ("completed", "cancelled"):
                to_remove.append(dl_id)

        for dl_id in to_remove:
            # Cancel YouTube downloader jika ada
            item = self.engine._downloads.get(dl_id)
            if item and hasattr(item, '_yt_downloader') and item._yt_downloader:
                try:
                    item._yt_downloader.cancel()
                except Exception:
                    pass

            row = self._rows.pop(dl_id, None)
            if row:
                self.listbox.remove(row)
            self.engine.clear_download(dl_id)

    # ──────────────────────────────────────────────────────
    # Callbacks from engine (any thread)
    # ──────────────────────────────────────────────────────

    def _cb_update(self, dl_data):
        GLib.idle_add(self._gtk_update, dl_data)

    def _cb_complete(self, dl_data):
        GLib.idle_add(self._gtk_update, dl_data)

    def _gtk_update(self, dl_data):
        row = self._rows.get(dl_data["id"])
        if row:
            row.update(dl_data)
        self._gtk_stats()

    def _gtk_stats(self):
        downloads = self.engine.get_all_downloads()
        active = [d for d in downloads if d["status"] == "downloading"]
        total_speed = sum(d["speed"] for d in active)

        self.lbl_active.set_text(str(len(active)))
        self.lbl_speed.set_text(
            "{}/s".format(format_size(total_speed))
            if total_speed else "0 B/s"
        )
        self.lbl_total.set_text(str(len(downloads)))

    def start_youtube_from_extension(self, url, quality=None):
        """
        Handle YouTube download dari extension overlay.
        Dipanggil dari _handle_message via GLib.idle_add.

        Jika quality sudah dipilih dari overlay → langsung download.
        Jika quality None → tampilkan dialog pilih kualitas.
        """
        if not check_ytdlp():
            print("[FastDM] yt-dlp not found, skipping YouTube",
                  file=sys.stderr)
            return

        if quality:
            # Quality sudah dipilih dari overlay → langsung download
            self._start_yt_direct(url, quality)
        else:
            # Tidak ada quality → tampilkan dialog
            self._handle_youtube(url)

    def _start_yt_direct(self, url, quality):
        """
        Download YouTube langsung tanpa dialog.
        Dipanggil saat user pilih kualitas dari overlay browser.
        """
        self.url_entry.set_placeholder_text("Fetching video info...")
        self.url_entry.set_sensitive(False)

        def _fetch():
            info = get_video_info(url)
            GLib.idle_add(self._do_yt_direct, url, quality, info)

        threading.Thread(target=_fetch, daemon=True).start()

    def _do_yt_direct(self, url, quality, info):
        """Mulai download YouTube setelah info didapat."""
        self.url_entry.set_placeholder_text("Paste download URL here...")
        self.url_entry.set_sensitive(True)

        if info and info.get("type") == "video":
            self._start_yt_download(url, info, quality, None)
        elif info and info.get("type") == "playlist":
            # Playlist dari overlay → download semua dengan quality terpilih
            entries = info.get("entries", [])
            for entry in entries:
                entry_url = entry.get("url", "")
                if entry_url:
                    self._start_yt_download(entry_url, {
                        "title": entry.get("title", "YouTube video"),
                        "type": "video",
                    }, quality, None)
        else:
            # Info gagal → coba download langsung tanpa info
            self._start_yt_download(url, {
                "title": "YouTube video",
                "type": "video",
            }, quality, None)

    def _on_quit(self, *_):
        self.engine.shutdown()
        Gtk.main_quit()
        return False
