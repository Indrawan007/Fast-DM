pub const THEME_CSS: &str = r#"
/* Window */
window { background-color: #11111b; }

/* Header */
.header-box {
    background-color: #181825;
    padding: 14px 20px;
    border-bottom: 1px solid rgba(137, 180, 250, 0.15);
}
.header-title {
    color: #cdd6f4;
    font-size: 16px;
    font-weight: 800;
}
.header-subtitle {
    color: #585b70;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1px;
}

/* Toolbar */
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
}
.url-entry:focus {
    border-color: #89b4fa;
}

/* Buttons */
.btn-download {
    padding: 10px 20px;
    background-color: #89b4fa;
    color: #11111b;
    border: none;
    border-radius: 10px;
    font-weight: 800;
    font-size: 13px;
}
.btn-download:hover { background-color: #b4d0fb; }

.btn-clear {
    padding: 10px 16px;
    background-color: transparent;
    color: #6c7086;
    border: 1px solid #313244;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 600;
}
.btn-clear:hover { background-color: #1e1e2e; color: #a6adc8; }

/* Download Card */
.download-card {
    background-color: #1e1e2e;
    border-radius: 12px;
    border: 1px solid #313244;
    margin: 4px 12px;
}
.download-card:hover { background-color: #232336; border-color: #45475a; }

.card-inner { padding: 14px 16px; }

/* Labels */
.filename-label { color: #cdd6f4; font-weight: 700; font-size: 13px; }
.detail-label   { color: #585b70; font-size: 11px; font-weight: 500; }
.detail-speed   { color: #89b4fa; font-size: 11px; font-weight: 700; }
.progress-text  { color: #a6adc8; font-size: 11px; font-weight: 700; }
.error-label    { color: #f38ba8; font-size: 11px; font-style: italic; }

/* Badges */
.badge {
    font-size: 9px; font-weight: 800;
    padding: 3px 10px; border-radius: 20px;
    letter-spacing: 0.8px;
}
.badge-downloading { background-color: rgba(137,180,250,0.15); color: #89b4fa; }
.badge-resolving   { background-color: rgba(116,199,236,0.15); color: #74c7ec; }
.badge-completed   { background-color: rgba(166,227,161,0.15); color: #a6e3a1; }
.badge-error       { background-color: rgba(243,139,168,0.15); color: #f38ba8; }
.badge-paused      { background-color: rgba(250,179,135,0.15); color: #fab387; }
.badge-cancelled   { background-color: rgba(108,112,134,0.15); color: #6c7086; }

/* Progress Bar */
progressbar trough {
    min-height: 4px; border-radius: 2px; background-color: #313244;
}
progressbar progress {
    min-height: 4px; border-radius: 2px; background-color: #89b4fa;
}
progressbar.completed progress { background-color: #a6e3a1; }
progressbar.error progress     { background-color: #f38ba8; }
progressbar.paused progress    { background-color: #fab387; }

/* Action Buttons */
.btn-action {
    padding: 5px 14px; border-radius: 8px;
    font-size: 11px; font-weight: 700; border: none;
}
.btn-pause  { background-color: rgba(250,179,135,0.12); color: #fab387; }
.btn-resume { background-color: rgba(166,227,161,0.12); color: #a6e3a1; }
.btn-retry  { background-color: rgba(137,180,250,0.12); color: #89b4fa; }
.btn-cancel { background-color: rgba(243,139,168,0.08); color: #f38ba8; }
.btn-open   { background-color: #89b4fa; color: #11111b; }
.btn-remove { background-color: transparent; color: #585b70; border: 1px solid #313244; }

/* Stats Bar */
.stats-box {
    background-color: #181825; padding: 8px 20px;
    border-top: 1px solid rgba(69,71,90,0.5);
}
.stats-label { color: #45475a; font-size: 11px; font-weight: 600; }
.stats-value { color: #6c7086; font-size: 11px; font-weight: 700; }
.stats-speed { color: #89b4fa; font-size: 11px; font-weight: 800; }

/* Placeholder */
.ph-icon  { color: #313244; font-size: 48px; }
.ph-title { color: #45475a; font-size: 15px; font-weight: 700; }
.ph-sub   { color: #313244; font-size: 12px; }

/* Scrollbar */
scrolledwindow scrollbar slider { background-color: #313244; border-radius: 10px; min-width: 4px; }
scrolledwindow scrollbar slider:hover { background-color: #45475a; }
"#;
