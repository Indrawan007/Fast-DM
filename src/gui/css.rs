pub const THEME_CSS: &str = r#"

/* ══════════════════════════════════════════
   SEMUA rule di-scope ke .fast-dm-window
   agar TIDAK mempengaruhi aplikasi lain
   ══════════════════════════════════════════ */

/* Window */
.fast-dm-window {
    background-color: #11111b;
}

/* Header */
.fast-dm-window .header-box {
    background-color: #181825;
    padding: 14px 20px;
    border-bottom: 1px solid rgba(137, 180, 250, 0.15);
}
.fast-dm-window .header-title {
    color: #cdd6f4;
    font-size: 16px;
    font-weight: 800;
}
.fast-dm-window .header-subtitle {
    color: #585b70;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1px;
}

/* Toolbar */
.fast-dm-window .toolbar-box {
    background-color: #181825;
    padding: 10px 16px;
    border-bottom: 1px solid rgba(69, 71, 90, 0.5);
}
.fast-dm-window .url-entry {
    background-color: #1e1e2e;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 13px;
}
.fast-dm-window .url-entry:focus {
    border-color: #89b4fa;
}

/* Buttons */
.fast-dm-window .btn-download {
    padding: 10px 20px;
    background-color: #89b4fa;
    color: #11111b;
    border: none;
    border-radius: 10px;
    font-weight: 800;
    font-size: 13px;
}
.fast-dm-window .btn-download:hover {
    background-color: #b4d0fb;
}

.fast-dm-window .btn-clear {
    padding: 10px 16px;
    background-color: transparent;
    color: #6c7086;
    border: 1px solid #313244;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 600;
}
.fast-dm-window .btn-clear:hover {
    background-color: #1e1e2e;
    color: #a6adc8;
}

/* Download Card */
.fast-dm-window .download-card {
    background-color: #1e1e2e;
    border-radius: 12px;
    border: 1px solid #313244;
    margin: 4px 12px;
}
.fast-dm-window .download-card:hover {
    background-color: #232336;
    border-color: #45475a;
}

.fast-dm-window .card-inner {
    padding: 14px 16px;
}

/* Labels */
.fast-dm-window .filename-label {
    color: #cdd6f4;
    font-weight: 700;
    font-size: 13px;
}
.fast-dm-window .detail-label {
    color: #585b70;
    font-size: 11px;
    font-weight: 500;
}
.fast-dm-window .detail-speed {
    color: #89b4fa;
    font-size: 11px;
    font-weight: 700;
}
.fast-dm-window .progress-text {
    color: #a6adc8;
    font-size: 11px;
    font-weight: 700;
}
.fast-dm-window .error-label {
    color: #f38ba8;
    font-size: 11px;
    font-style: italic;
}

/* v2.3.0 (M10): info non-error (mis. "Merging video + audio…") — tidak lagi
   menumpang error_msg merah */
.fast-dm-window .info-label {
    color: #89dceb;
    font-size: 11px;
    font-weight: 500;
}


/* Badges */
.fast-dm-window .badge {
    font-size: 9px;
    font-weight: 800;
    padding: 3px 10px;
    border-radius: 20px;
    letter-spacing: 0.8px;
}
.fast-dm-window .badge-downloading {
    background-color: rgba(137, 180, 250, 0.15);
    color: #89b4fa;
}
.fast-dm-window .badge-resolving {
    background-color: rgba(116, 199, 236, 0.15);
    color: #74c7ec;
}
.fast-dm-window .badge-completed {
    background-color: rgba(166, 227, 161, 0.15);
    color: #a6e3a1;
}
.fast-dm-window .badge-error {
    background-color: rgba(243, 139, 168, 0.15);
    color: #f38ba8;
}
.fast-dm-window .badge-paused {
    background-color: rgba(250, 179, 135, 0.15);
    color: #fab387;
}
.fast-dm-window .badge-cancelled {
    background-color: rgba(108, 112, 134, 0.15);
    color: #6c7086;
}

/* Progress Bar — scoped */
.fast-dm-window progressbar trough {
    min-height: 4px;
    border-radius: 2px;
    background-color: #313244;
}
.fast-dm-window progressbar progress {
    min-height: 4px;
    border-radius: 2px;
    background-color: #89b4fa;
}
.fast-dm-window progressbar.completed progress {
    background-color: #a6e3a1;
}
.fast-dm-window progressbar.error progress {
    background-color: #f38ba8;
}
.fast-dm-window progressbar.paused progress {
    background-color: #fab387;
}

/* Action Buttons */
.fast-dm-window .btn-action {
    padding: 5px 14px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: 700;
    border: none;
}
.fast-dm-window .btn-pause {
    background-color: rgba(250, 179, 135, 0.12);
    color: #fab387;
}
.fast-dm-window .btn-pause:hover {
    background-color: rgba(250, 179, 135, 0.25);
}
.fast-dm-window .btn-resume {
    background-color: rgba(166, 227, 161, 0.12);
    color: #a6e3a1;
}
.fast-dm-window .btn-resume:hover {
    background-color: rgba(166, 227, 161, 0.25);
}
.fast-dm-window .btn-retry {
    background-color: rgba(137, 180, 250, 0.12);
    color: #89b4fa;
}
.fast-dm-window .btn-retry:hover {
    background-color: rgba(137, 180, 250, 0.25);
}
.fast-dm-window .btn-cancel {
    background-color: rgba(243, 139, 168, 0.08);
    color: #f38ba8;
}
.fast-dm-window .btn-cancel:hover {
    background-color: rgba(243, 139, 168, 0.2);
}
.fast-dm-window .btn-open {
    background-color: #89b4fa;
    color: #11111b;
}
.fast-dm-window .btn-open:hover {
    background-color: #b4d0fb;
}
.fast-dm-window .btn-remove {
    background-color: transparent;
    color: #585b70;
    border: 1px solid #313244;
}
.fast-dm-window .btn-remove:hover {
    background-color: #1e1e2e;
    color: #6c7086;
}

/* Stats Bar */
.fast-dm-window .stats-box {
    background-color: #181825;
    padding: 8px 20px;
    border-top: 1px solid rgba(69, 71, 90, 0.5);
}
.fast-dm-window .stats-label {
    color: #45475a;
    font-size: 11px;
    font-weight: 600;
}
.fast-dm-window .stats-value {
    color: #6c7086;
    font-size: 11px;
    font-weight: 700;
}
.fast-dm-window .stats-speed {
    color: #89b4fa;
    font-size: 11px;
    font-weight: 800;
}

/* Placeholder */
.fast-dm-window .ph-icon {
    color: #313244;
    font-size: 48px;
}
.fast-dm-window .ph-title {
    color: #45475a;
    font-size: 15px;
    font-weight: 700;
}
.fast-dm-window .ph-sub {
    color: #313244;
    font-size: 12px;
}

/* Download list background */
.fast-dm-window .download-list {
    background-color: #11111b;
}

/* Scrollbar — scoped */
.fast-dm-window scrolledwindow scrollbar slider {
    background-color: #313244;
    border-radius: 10px;
    min-width: 4px;
}
.fast-dm-window scrolledwindow scrollbar slider:hover {
    background-color: #45475a;
}

/* ═══ Dialog (settings, YouTube, konfirmasi) ═══
   Dialog diberi class .fast-dm-window juga supaya tema sama dengan
   window utama (B2). Rule di bawah menyamakan kontrol bawaan GTK. */
.fast-dm-window label {
    color: #cdd6f4;
}
.fast-dm-window entry,
.fast-dm-window spinbutton {
    background-color: #1e1e2e;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 8px;
    padding: 6px 10px;
}
.fast-dm-window entry:focus,
.fast-dm-window spinbutton:focus {
    border-color: #89b4fa;
}

/* Tombol yang tidak relevan untuk status saat ini (B4) */
.fast-dm-window button:disabled {
    opacity: 0.35;
}
"#;
