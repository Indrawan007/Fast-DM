# gui/manager.py

import os
import subprocess
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib, Gdk, Pango

from engine import DownloadEngine, DownloadStatus, Config
from engine.utils import format_size, format_speed, format_eta


CSS = b"""
window { background-color: #1e1e2e; }

.header {
    background-color: #181825;
    padding: 10px 16px;
    border-bottom: 1px solid #313244;
}
.header-title {
    color: #cdd6f4;
    font-size: 15px;
    font-weight: bold;
}

.toolbar {
    background-color: #181825;
    padding: 8px 12px;
    border-bottom: 1px solid #313244;
}

list { background-color: #1e1e2e; }

row {
    background-color: #313244;
    border-radius: 8px;
    margin: 3px 8px;
    padding: 0;
}
row:hover { background-color: #3d3f55; }

.filename-label {
    color: #cdd6f4;
    font-weight: 600;
    font-size: 13px;
}
.status-label {
    font-size: 10px;
    font-weight: bold;
    padding: 2px 7px;
    border-radius: 4px;
    background-color: #45475a;
    color: #a6adc8;
}
.status-downloading { background-color: #1e4d8c; color: #89b4fa; }
.status-completed   { background-color: #1e4d2e; color: #a6e3a1; }
.status-error       { background-color: #4d1e1e; color: #f38ba8; }
.status-paused      { background-color: #4d3a1e; color: #fab387; }
.status-cancelled   { background-color: #3a3a3a; color: #6c7086; }
.status-resolving   { background-color: #1e3a4d; color: #74c7ec; }

.detail-label {
    color: #6c7086;
    font-size: 11px;
}
.error-label {
    color: #f38ba8;
    font-size: 11px;
    font-style: italic;
}
.retry-label {
    color: #fab387;
    font-size: 11px;
}

progressbar trough {
    min-height: 6px;
    border-radius: 3px;
    background-color: #45475a;
}
progressbar progress {
    min-height: 6px;
    border-radius: 3px;
    background-color: #89b4fa;
}
progressbar.error progress {
    background-color: #f38ba8;
}
progressbar.completed progress {
    background-color: #a6e3a1;
}

.btn-pause  { padding: 3px 10px; font-size: 11px;
              background: #313244; color: #fab387;
              border: 1px solid #fab387; border-radius: 4px; }
.btn-resume { padding: 3px 10px; font-size: 11px;
              background: #313244; color: #a6e3a1;
              border: 1px solid #a6e3a1; border-radius: 4px; }
.btn-retry  { padding: 3px 10px; font-size: 11px;
              background: #313244; color: #89b4fa;
              border: 1px solid #89b4fa; border-radius: 4px;
              font-weight: bold; }
.btn-retry:hover { background: #1e4d8c; }
.btn-cancel { padding: 3px 10px; font-size: 11px;
              background: #313244; color: #f38ba8;
              border: 1px solid #f38ba8; border-radius: 4px; }
.btn-open   { padding: 3px 10px; font-size: 11px;
              background: #89b4fa; color: #1e1e2e;
              border: none; border-radius: 4px; font-weight: bold; }
.btn-remove { padding: 3px 10px; font-size: 11px;
              background: #313244; color: #6c7086;
              border: 1px solid #45475a; border-radius: 4px; }

.url-entry {
    background-color: #313244;
    color: #cdd6f4;
    border: 1px solid #45475a;
    border-radius: 6px;
    padding: 7px 11px;
    font-size: 13px;
}
.url-entry:focus { border-color: #89b4fa; }

.btn-add {
    padding: 7px 16px;
    background: #89b4fa;
    color: #1e1e2e;
    border: none;
    border-radius: 6px;
    font-weight: bold;
    font-size: 13px;
}
.btn-add:hover { background: #b4d0fb; }

.btn-clear {
    padding: 7px 14px;
    background: #313244;
    color: #a6adc8;
    border: 1px solid #45475a;
    border-radius: 6px;
    font-size: 12px;
}

.statsbar {
    background-color: #181825;
    border-top: 1px solid #313244;
    padding: 5px 16px;
}
.stats-label { color: #6c7086; font-size: 11px; }
"""


class DownloadRow(Gtk.ListBoxRow):

    def __init__(self, dl_data):
        super().__init__()
        self.dl_id = dl_data["id"]

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
        outer.set_margin_top(8)
        outer.set_margin_bottom(8)
        outer.set_margin_start(12)
        outer.set_margin_end(12)

        # ── Row 1: filename + status ──
        row1 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)

        self.filename_lbl = Gtk.Label(xalign=0.0)
        self.filename_lbl.set_markup(
            "<b>{}</b>".format(GLib.markup_escape_text(dl_data["filename"]))
        )
        self.filename_lbl.set_ellipsize(Pango.EllipsizeMode.MIDDLE)
        self.filename_lbl.set_hexpand(True)
        self.filename_lbl.get_style_context().add_class("filename-label")

        self.status_lbl = Gtk.Label(label=dl_data["status"].upper())
        self.status_lbl.get_style_context().add_class("status-label")

        row1.pack_start(self.filename_lbl, True, True, 0)
        row1.pack_end(self.status_lbl, False, False, 0)

        # ── Row 2: progress bar ──
        self.progress_bar = Gtk.ProgressBar()
        self.progress_bar.set_show_text(True)
        self._set_progress(dl_data)

        # ── Row 3: details ──
        row3 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=16)
        self.size_lbl  = Gtk.Label(xalign=0.0)
        self.speed_lbl = Gtk.Label(xalign=0.0)
        self.eta_lbl   = Gtk.Label(xalign=0.0)
        self.conn_lbl  = Gtk.Label(xalign=0.0)
        for lbl in (self.size_lbl, self.speed_lbl,
                    self.eta_lbl, self.conn_lbl):
            lbl.get_style_context().add_class("detail-label")
            row3.pack_start(lbl, False, False, 0)
        self._set_details(dl_data)

        # ── Row 3b: error message (hidden by default) ──
        self.error_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self.error_icon = Gtk.Label(label="⚠")
        self.error_lbl = Gtk.Label(xalign=0.0)
        self.error_lbl.set_ellipsize(Pango.EllipsizeMode.END)
        self.error_lbl.set_hexpand(True)
        self.error_lbl.get_style_context().add_class("error-label")
        self.error_box.pack_start(self.error_icon, False, False, 0)
        self.error_box.pack_start(self.error_lbl, True, True, 0)
        self.error_box.set_no_show_all(True)
        self.error_box.hide()

        # ── Row 4: buttons ──
        row4 = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        row4.set_halign(Gtk.Align.END)

        self.pause_btn  = Gtk.Button(label="⏸ Pause")
        self.resume_btn = Gtk.Button(label="▶ Resume")
        self.retry_btn  = Gtk.Button(label="🔄 Retry")
        self.cancel_btn = Gtk.Button(label="✕ Cancel")
        self.open_btn   = Gtk.Button(label="📂 Open Folder")
        self.remove_btn = Gtk.Button(label="🗑 Remove")

        self.pause_btn.get_style_context().add_class("btn-pause")
        self.resume_btn.get_style_context().add_class("btn-resume")
        self.retry_btn.get_style_context().add_class("btn-retry")
        self.cancel_btn.get_style_context().add_class("btn-cancel")
        self.open_btn.get_style_context().add_class("btn-open")
        self.remove_btn.get_style_context().add_class("btn-remove")

        for btn in (self.retry_btn, self.resume_btn, self.pause_btn,
                    self.cancel_btn, self.open_btn, self.remove_btn):
            row4.pack_start(btn, False, False, 0)

        # ── Assemble ──
        outer.pack_start(row1, False, False, 0)
        outer.pack_start(self.progress_bar, False, False, 0)
        outer.pack_start(row3, False, False, 0)
        outer.pack_start(self.error_box, False, False, 0)
        outer.pack_start(row4, False, False, 0)
        self.add(outer)

        self._update_buttons(dl_data["status"])

    def _set_progress(self, d):
        pct = float(d["progress"])
        self.progress_bar.set_fraction(min(pct / 100.0, 1.0))
        self.progress_bar.set_text("{:.1f}%".format(pct))

        ctx = self.progress_bar.get_style_context()
        ctx.remove_class("error")
        ctx.remove_class("completed")
        if d["status"] == "error":
            ctx.add_class("error")
        elif d["status"] == "completed":
            ctx.add_class("completed")

    def _set_details(self, d):
        self.size_lbl.set_text(
            "{} / {}".format(d["downloaded_fmt"], d["total_size_fmt"])
        )
        self.speed_lbl.set_text(d["speed_fmt"])
        self.eta_lbl.set_text("ETA: {}".format(d["eta_fmt"]))
        self.conn_lbl.set_text("CN: {}".format(d["connections"]))

    def _update_buttons(self, status):
        active    = status in ("downloading", "resolving")
        paused    = status == "paused"
        error     = status == "error"
        done      = status == "completed"
        cancelled = status == "cancelled"
        stoppable = active or paused

        self.pause_btn.set_visible(active)
        self.resume_btn.set_visible(paused)
        self.retry_btn.set_visible(error)
        self.cancel_btn.set_visible(stoppable or error)
        self.open_btn.set_visible(done)
        self.remove_btn.set_visible(done or cancelled or error)

    def update(self, d):
        self._set_progress(d)
        self._set_details(d)

        # Update filename (bisa berubah setelah resolve)
        self.filename_lbl.set_markup(
            "<b>{}</b>".format(GLib.markup_escape_text(d["filename"]))
        )

        # Status badge
        status = d["status"]
        self.status_lbl.set_text(status.upper())

        ctx = self.status_lbl.get_style_context()
        for cls in ("status-downloading", "status-completed",
                    "status-error", "status-paused", "status-cancelled",
                    "status-resolving"):
            ctx.remove_class(cls)

        status_map = {
            "downloading": "status-downloading",
            "completed":   "status-completed",
            "error":       "status-error",
            "paused":      "status-paused",
            "cancelled":   "status-cancelled",
            "resolving":   "status-resolving",
        }
        cls = status_map.get(status)
        if cls:
            ctx.add_class(cls)

        # Error message
        err = d.get("error_msg", "")
        if err and status == "error":
            self.error_lbl.set_text(err)
            self.error_box.set_no_show_all(False)
            self.error_box.show_all()
        elif err and status == "downloading":
            # Retry message
            self.error_lbl.set_text(err)
            self.error_lbl.get_style_context().remove_class("error-label")
            self.error_lbl.get_style_context().add_class("retry-label")
            self.error_box.set_no_show_all(False)
            self.error_box.show_all()
        else:
            self.error_box.hide()
            self.error_lbl.get_style_context().remove_class("retry-label")
            self.error_lbl.get_style_context().add_class("error-label")

        self._update_buttons(status)


class ManagerWindow(Gtk.Window):

    def __init__(self, engine):
        super().__init__(title="⚡ Fast Download Manager")
        self.engine = engine
        self._rows  = {}

        self.set_default_size(760, 560)
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
        provider.load_from_data(CSS)
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

        # Header
        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        header.get_style_context().add_class("header")
        title = Gtk.Label(label="⚡  Fast Download Manager")
        title.get_style_context().add_class("header-title")
        header.pack_start(title, False, False, 0)
        root.pack_start(header, False, False, 0)

        # URL toolbar
        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        toolbar.get_style_context().add_class("toolbar")

        self.url_entry = Gtk.Entry()
        self.url_entry.set_placeholder_text(
            "Paste URL here, or use the Chrome extension…")
        self.url_entry.set_hexpand(True)
        self.url_entry.get_style_context().add_class("url-entry")
        self.url_entry.connect("activate", self._on_add_url)

        add_btn = Gtk.Button(label="⬇  Download")
        add_btn.get_style_context().add_class("btn-add")
        add_btn.connect("clicked", self._on_add_url)

        clear_btn = Gtk.Button(label="🗑  Clear Done")
        clear_btn.get_style_context().add_class("btn-clear")
        clear_btn.connect("clicked", self._on_clear_done)

        toolbar.pack_start(self.url_entry, True, True, 0)
        toolbar.pack_start(add_btn, False, False, 0)
        toolbar.pack_start(clear_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        # Download list
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)

        self.listbox = Gtk.ListBox()
        self.listbox.set_selection_mode(Gtk.SelectionMode.NONE)

        placeholder = Gtk.Label()
        placeholder.set_markup(
            "\n\n<span foreground='#45475a' size='large'>"
            "No downloads yet\n\n"
            "<small>Paste a URL above, right-click a link in Chrome,\n"
            "or click the ⚡ extension icon.</small></span>\n\n")
        placeholder.set_justify(Gtk.Justification.CENTER)
        self.listbox.set_placeholder(placeholder)

        scroll.add(self.listbox)
        root.pack_start(scroll, True, True, 0)

        # Stats bar
        statsbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=20)
        statsbar.get_style_context().add_class("statsbar")
        self.lbl_active = Gtk.Label(label="Active: 0")
        self.lbl_speed  = Gtk.Label(label="Speed: 0 B/s")
        self.lbl_total  = Gtk.Label(label="Total: 0")
        for lbl in (self.lbl_active, self.lbl_speed, self.lbl_total):
            lbl.get_style_context().add_class("stats-label")
            statsbar.pack_start(lbl, False, False, 0)
        root.pack_start(statsbar, False, False, 0)

        self.add(root)

    def _on_add_url(self, _w):
        url = self.url_entry.get_text().strip()
        if not url:
            return
        if not url.startswith(("http://", "https://", "ftp://")):
            url = "https://" + url
        dl_id = self.engine.add_download(url)
        self.url_entry.set_text("")
        self._add_row(dl_id)

    def add_download_from_extension(self, url, filename=None, headers=None):
        dl_id = self.engine.add_download(
            url, filename=filename, headers=headers)
        GLib.idle_add(self._add_row, dl_id)
        return dl_id

    def _add_row(self, dl_id):
        dl_data = self.engine.get_download(dl_id)
        if not dl_data:
            return

        row = DownloadRow(dl_data)

        # ── Pause: hentikan sementara ──
        row.pause_btn.connect(
            "clicked", lambda _b: self.engine.pause_download(dl_id))

        # ── Resume: lanjutkan dari pause/error ──
        row.resume_btn.connect(
            "clicked", lambda _b: self.engine.resume_download(dl_id))

        # ── Retry: coba ulang dari error ──
        row.retry_btn.connect(
            "clicked", lambda _b: self.engine.retry_download(dl_id))

        # ── Cancel: batalkan DAN hapus file partial ──
        def on_cancel(_b):
            self.engine.cancel_download(dl_id)
        row.cancel_btn.connect("clicked", on_cancel)

        # ── Open Folder: buka lokasi file ──
        row.open_btn.connect(
            "clicked", lambda _b, d=dl_data: self._open_folder(d))

        # ── Remove: hapus dari daftar, file TETAP ada ──
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
        """Remove row dari list. TIDAK hapus file yang sudah didownload."""
        row = self._rows.pop(dl_id, None)
        if row:
            self.listbox.remove(row)
        # Pakai clear_download, bukan remove_download
        self.engine.clear_download(dl_id)

    def _open_folder(self, dl_data):
        subprocess.Popen(
            ["xdg-open", dl_data["save_dir"]],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def _on_clear_done(self, _w):
        """
        Hapus semua entri yang sudah COMPLETED atau CANCELLED dari daftar.
        File yang sudah didownload TETAP ADA di disk.
        """
        done_ids = []
        for dl_id in list(self._rows.keys()):
            d = self.engine.get_download(dl_id)
            if d and d["status"] in ("completed", "cancelled"):
                done_ids.append(dl_id)

        for dl_id in done_ids:
            row = self._rows.pop(dl_id, None)
            if row:
                self.listbox.remove(row)
            self.engine.clear_download(dl_id)

    # Engine callbacks
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
        self.lbl_active.set_text("Active: {}".format(len(active)))
        self.lbl_speed.set_text("Speed: {}".format(
            "{}/s".format(format_size(total_speed))
            if total_speed else "0 B/s"))
        self.lbl_total.set_text("Total: {}".format(len(downloads)))

    def _on_quit(self, *_):
        self.engine.shutdown()
        Gtk.main_quit()
        return False
