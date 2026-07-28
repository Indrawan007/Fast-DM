# engine/youtube.py
"""
YouTube Downloader proper untuk Fast DM.

Fitur:
- Auto-detect browser cookies (Thorium, Chrome, Chromium, Brave, dll)
- Fallback ke cookies.txt jika browser terkunci
- Export cookies dari browser otomatis
- Pilihan kualitas lengkap + audio only
- Download subtitle
- Download playlist support
- Progress tracking real-time
- Pause / Cancel support
- Resume jika terputus
"""

import subprocess
import threading
import json
import re
import os
import time
import sys
import signal
import shutil
from pathlib import Path


# ══════════════════════════════════════════════════════════
# Constants
# ══════════════════════════════════════════════════════════

YT_PATTERNS = [
    r'(?:https?://)?(?:www\.)?youtube\.com/watch\?v=[\w-]+',
    r'(?:https?://)?(?:www\.)?youtube\.com/shorts/[\w-]+',
    r'(?:https?://)?youtu\.be/[\w-]+',
    r'(?:https?://)?(?:www\.)?youtube\.com/embed/[\w-]+',
    r'(?:https?://)?(?:www\.)?youtube\.com/playlist\?list=[\w-]+',
    r'(?:https?://)?music\.youtube\.com/watch\?v=[\w-]+',
    r'(?:https?://)?(?:www\.)?youtube\.com/channel/[\w-]+',
    r'(?:https?://)?(?:www\.)?youtube\.com/@[\w-]+',
]

QUALITY_PRESETS = {
    "best_mp4":  {
        "label": "Best Quality (MP4)",
        "desc": "Highest resolution, MP4 format",
        "format": "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best",
        "ext": "mp4",
    },
    "best_any": {
        "label": "Best Quality (Any)",
        "desc": "Highest resolution, any format",
        "format": "bestvideo+bestaudio/best",
        "ext": "mkv",
    },
    "2160p": {
        "label": "4K Ultra HD",
        "desc": "3840×2160",
        "format": "bestvideo[height<=2160][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=2160]+bestaudio",
        "ext": "mp4",
    },
    "1440p": {
        "label": "2K QHD",
        "desc": "2560×1440",
        "format": "bestvideo[height<=1440][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=1440]+bestaudio",
        "ext": "mp4",
    },
    "1080p": {
        "label": "1080p Full HD",
        "desc": "1920×1080",
        "format": "bestvideo[height<=1080][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=1080]+bestaudio",
        "ext": "mp4",
    },
    "720p": {
        "label": "720p HD",
        "desc": "1280×720",
        "format": "bestvideo[height<=720][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=720]+bestaudio",
        "ext": "mp4",
    },
    "480p": {
        "label": "480p SD",
        "desc": "854×480",
        "format": "bestvideo[height<=480][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=480]+bestaudio",
        "ext": "mp4",
    },
    "360p": {
        "label": "360p Low",
        "desc": "640×360",
        "format": "bestvideo[height<=360][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=360]+bestaudio",
        "ext": "mp4",
    },
    "audio_best": {
        "label": "Audio — Best Quality",
        "desc": "M4A / highest bitrate",
        "format": "bestaudio[ext=m4a]/bestaudio",
        "ext": "m4a",
    },
    "audio_mp3": {
        "label": "Audio — MP3",
        "desc": "Converted to MP3 320kbps",
        "format": "bestaudio/best",
        "ext": "mp3",
        "postprocess": ["--extract-audio", "--audio-format", "mp3",
                        "--audio-quality", "0"],
    },
}

# Urutan tampil di UI
QUALITY_ORDER = [
    "best_mp4", "best_any",
    "2160p", "1440p", "1080p", "720p", "480p", "360p",
    "audio_best", "audio_mp3",
]

# Browser yang didukung untuk cookies
BROWSER_CANDIDATES = [
    ("thorium",  "~/.config/thorium"),
    ("chromium", "~/.config/chromium"),
    ("chrome",   "~/.config/google-chrome"),
    ("brave",    "~/.config/BraveSoftware/Brave-Browser"),
    ("edge",     "~/.config/microsoft-edge"),
    ("vivaldi",  "~/.config/vivaldi"),
    ("opera",    "~/.config/opera"),
    ("firefox",  "~/.mozilla/firefox"),
]

COOKIES_DIR = os.path.expanduser("~/.config/fast-dm")
COOKIES_FILE = os.path.join(COOKIES_DIR, "cookies.txt")


# ══════════════════════════════════════════════════════════
# Utility Functions
# ══════════════════════════════════════════════════════════

def is_youtube_url(url):
    """Cek apakah URL YouTube."""
    if not url:
        return False
    for pattern in YT_PATTERNS:
        if re.search(pattern, url):
            return True
    return False


def is_playlist_url(url):
    """Cek apakah URL adalah YouTube playlist."""
    if not url:
        return False
    return "playlist?list=" in url or "&list=" in url


def check_ytdlp():
    """Cek apakah yt-dlp terinstall."""
    try:
        result = subprocess.run(
            ["yt-dlp", "--version"],
            capture_output=True, text=True, timeout=5
        )
        return result.returncode == 0
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def detect_browser():
    """
    Deteksi browser untuk cookies.
    
    PENTING: yt-dlp hanya support browser tertentu.
    Untuk Thorium/Helium, pakai 'chromium' karena format
    database cookies-nya identik.
    
    yt-dlp supported: brave, chrome, chromium, edge,
                      firefox, opera, safari, vivaldi, whale
    """
    # Mapping: (config_dir, ytdlp_name)
    # ytdlp_name = nama yang dikenali yt-dlp
    candidates = [
        ("~/.config/thorium",                     "chromium"),  # Thorium → chromium
        ("~/.config/chromium",                     "chromium"),
        ("~/.config/google-chrome",                "chrome"),
        ("~/.config/BraveSoftware/Brave-Browser",  "brave"),
        ("~/.config/microsoft-edge",               "edge"),
        ("~/.config/vivaldi",                      "vivaldi"),
        ("~/.config/opera",                        "opera"),
        ("~/.mozilla/firefox",                     "firefox"),
    ]

    for path, ytdlp_name in candidates:
        expanded = os.path.expanduser(path)
        if os.path.isdir(expanded):
            return ytdlp_name, expanded

    return None, None


def get_all_browsers():
    """Return list semua browser yang terdeteksi + nama yt-dlp-nya."""
    found = []
    candidates = [
        ("Thorium / Helium",  "~/.config/thorium",                    "chromium"),
        ("Chromium",          "~/.config/chromium",                    "chromium"),
        ("Google Chrome",     "~/.config/google-chrome",              "chrome"),
        ("Brave",             "~/.config/BraveSoftware/Brave-Browser","brave"),
        ("Microsoft Edge",    "~/.config/microsoft-edge",             "edge"),
        ("Vivaldi",           "~/.config/vivaldi",                    "vivaldi"),
        ("Opera",             "~/.config/opera",                      "opera"),
        ("Firefox",           "~/.mozilla/firefox",                   "firefox"),
    ]
    for display_name, path, ytdlp_name in candidates:
        if os.path.isdir(os.path.expanduser(path)):
            found.append({
                "name": display_name,
                "ytdlp": ytdlp_name,
                "path": path,
            })
    return found


def export_cookies(browser_name=None):
    """
    Export cookies dari browser ke cookies.txt.
    """
    if not browser_name:
        browser_name, _ = detect_browser()
    if not browser_name:
        return False

    os.makedirs(COOKIES_DIR, exist_ok=True)

    try:
        cmd = [
            "yt-dlp",
            "--cookies-from-browser", browser_name,
            "--cookies", COOKIES_FILE,
            "--skip-download",
            "--no-warnings",
            "https://www.youtube.com",
        ]

        print("[FastDM] Exporting cookies from: {}".format(browser_name),
              file=sys.stderr)

        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=30
        )

        if os.path.exists(COOKIES_FILE) and os.path.getsize(COOKIES_FILE) > 100:
            print("[FastDM] Cookies exported OK", file=sys.stderr)
            return True
        else:
            print("[FastDM] Cookie export failed: {}".format(
                result.stderr[:200]), file=sys.stderr)

    except Exception as e:
        print("[FastDM] Cookie export error: {}".format(e), file=sys.stderr)

    return False


def _build_cookie_args():
    """
    Build args cookie untuk yt-dlp.
    
    Strategi:
    1. Cookies.txt yang sudah di-export (paling reliable)
    2. --cookies-from-browser (hanya jika browser tertutup)
    3. Tanpa cookies (mungkin gagal untuk beberapa video)
    """
    # 1. Coba cookies.txt yang masih fresh
    if os.path.exists(COOKIES_FILE):
        age = time.time() - os.path.getmtime(COOKIES_FILE)
        if age < 7200 and os.path.getsize(COOKIES_FILE) > 100:
            print("[FastDM] Using cookies.txt (age: {:.0f}s)".format(age),
                  file=sys.stderr)
            return ["--cookies", COOKIES_FILE]

    # 2. Coba dari browser
    browser, _ = detect_browser()
    if browser:
        print("[FastDM] Using cookies from browser: {}".format(browser),
              file=sys.stderr)
        return ["--cookies-from-browser", browser]

    # 3. Tanpa cookies
    print("[FastDM] No cookies available", file=sys.stderr)
    return []


# ══════════════════════════════════════════════════════════
# Video Info
# ══════════════════════════════════════════════════════════

def get_video_info(url):
    """
    Ambil info video YouTube.
    Mencoba dengan cookies, fallback tanpa cookies.
    """
    # Coba export cookies dulu (background)
    if not os.path.exists(COOKIES_FILE):
        try:
            export_cookies()
        except Exception:
            pass

    cookie_args = _build_cookie_args()

    cmd = [
        "yt-dlp",
        "--dump-json",
        "--no-download",
        "--no-warnings",
        "--socket-timeout", "15",
    ] + cookie_args

    # Playlist handling
    if is_playlist_url(url):
        cmd.append("--flat-playlist")
    else:
        cmd.append("--no-playlist")

    cmd.append(url)

    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=60
        )

        if result.returncode != 0:
            stderr = result.stderr[:500]
            print("[FastDM] yt-dlp error: {}".format(stderr), file=sys.stderr)

            # Jika cookie error, coba tanpa cookies
            if "Sign in" in stderr or "cookies" in stderr.lower():
                print("[FastDM] Retrying without cookies...", file=sys.stderr)
                cmd2 = [
                    "yt-dlp", "--dump-json", "--no-download",
                    "--no-playlist", "--no-warnings",
                    "--socket-timeout", "15", url
                ]
                result = subprocess.run(
                    cmd2, capture_output=True, text=True, timeout=60
                )
                if result.returncode != 0:
                    return None

        # Playlist: multiple JSON objects
        if is_playlist_url(url):
            return _parse_playlist_info(result.stdout, url)

        data = json.loads(result.stdout)
        return _parse_single_video(data, url)

    except subprocess.TimeoutExpired:
        print("[FastDM] yt-dlp timeout", file=sys.stderr)
        return None
    except json.JSONDecodeError as e:
        print("[FastDM] JSON error: {}".format(e), file=sys.stderr)
        return None
    except Exception as e:
        print("[FastDM] Error: {}".format(e), file=sys.stderr)
        return None


def _parse_single_video(data, url):
    """Parse info satu video."""
    formats = _parse_formats(data.get("formats", []))

    duration = data.get("duration", 0)
    dur_str = ""
    if duration:
        m, s = divmod(int(duration), 60)
        h, m = divmod(m, 60)
        if h:
            dur_str = "{}:{:02d}:{:02d}".format(h, m, s)
        else:
            dur_str = "{}:{:02d}".format(m, s)

    return {
        "type": "video",
        "title": data.get("title", "Unknown"),
        "duration": duration,
        "duration_str": dur_str,
        "thumbnail": data.get("thumbnail", ""),
        "uploader": data.get("uploader", ""),
        "view_count": data.get("view_count", 0),
        "upload_date": data.get("upload_date", ""),
        "description": (data.get("description", "") or "")[:200],
        "formats": formats,
        "url": url,
        "has_subtitles": bool(data.get("subtitles")),
        "available_subs": list((data.get("subtitles") or {}).keys())[:10],
    }


def _parse_playlist_info(stdout, url):
    """Parse info playlist."""
    entries = []
    for line in stdout.strip().split('\n'):
        if not line.strip():
            continue
        try:
            item = json.loads(line)
            entries.append({
                "title": item.get("title", "Unknown"),
                "url": item.get("url") or item.get("webpage_url", ""),
                "duration": item.get("duration", 0),
                "id": item.get("id", ""),
            })
        except json.JSONDecodeError:
            continue

    if not entries:
        return None

    return {
        "type": "playlist",
        "title": "YouTube Playlist ({} videos)".format(len(entries)),
        "entries": entries,
        "url": url,
        "count": len(entries),
    }


def _parse_formats(raw_formats):
    """Parse format list dari yt-dlp."""
    formats = []
    seen = set()

    for f in raw_formats:
        height = f.get("height")
        ext = f.get("ext", "?")
        vcodec = f.get("vcodec", "none")
        acodec = f.get("acodec", "none")
        filesize = f.get("filesize") or f.get("filesize_approx") or 0
        tbr = f.get("tbr") or 0
        fps = f.get("fps") or 0

        if vcodec == "none" and acodec == "none":
            continue

        if height and vcodec != "none":
            quality = "{}p".format(height)
        elif acodec != "none" and vcodec == "none":
            quality = "Audio"
        else:
            quality = f.get("format_note", "unknown")

        key = "{}-{}-{}".format(quality, ext, "v" if vcodec != "none" else "a")
        if key in seen:
            continue
        seen.add(key)

        note_parts = []
        if height:
            note_parts.append("{}p".format(height))
        if fps and fps > 30:
            note_parts.append("{}fps".format(fps))
        if tbr:
            note_parts.append("~{:.0f}kbps".format(tbr))
        if filesize:
            from engine.utils import format_size
            note_parts.append(format_size(filesize))

        formats.append({
            "id": f.get("format_id", ""),
            "quality": quality,
            "ext": ext,
            "filesize": filesize,
            "height": height or 0,
            "fps": fps,
            "note": " · ".join(note_parts),
            "has_video": vcodec != "none",
            "has_audio": acodec != "none",
        })

    formats.sort(key=lambda x: (
        0 if x["has_video"] else 1,
        -x["height"],
        -x.get("fps", 0),
    ))

    return formats


# ══════════════════════════════════════════════════════════
# Downloader
# ══════════════════════════════════════════════════════════

class YouTubeDownloader:
    """YouTube Downloader dengan progress, pause, cancel, resume."""

    def __init__(self, save_dir=None):
        from engine.config import Config
        cfg = Config()
        self.save_dir = save_dir or cfg.download_dir
        self._process = None
        self._cancelled = False
        self._paused = False
        self._url = None
        self._quality = None
        self._callbacks = {}

    def download(self, url, quality="best_mp4",
                 subtitle_lang=None,
                 on_progress=None, on_complete=None, on_error=None):
        """Mulai download YouTube video."""
        self._cancelled = False
        self._paused = False
        self._url = url
        self._quality = quality
        self._callbacks = {
            "progress": on_progress,
            "complete": on_complete,
            "error": on_error,
        }

        t = threading.Thread(
            target=self._worker,
            args=(url, quality, subtitle_lang),
            daemon=True,
            name="yt-dl"
        )
        t.start()

    def pause(self):
        """Pause download (kill yt-dlp, bisa resume karena partial file)."""
        self._paused = True
        self._kill()

    def resume(self):
        """Resume download."""
        if not self._paused or not self._url:
            return
        self._paused = False
        self.download(
            self._url, self._quality,
            on_progress=self._callbacks.get("progress"),
            on_complete=self._callbacks.get("complete"),
            on_error=self._callbacks.get("error"),
        )

    def cancel(self):
        """Cancel download."""
        self._cancelled = True
        self._kill()

    def _kill(self):
        """Kill yt-dlp process."""
        proc = self._process
        if proc and proc.poll() is None:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired, OSError):
                try:
                    os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                except (ProcessLookupError, OSError):
                    pass
            self._process = None

    def _worker(self, url, quality, subtitle_lang):
        """Worker thread download."""
        on_progress = self._callbacks.get("progress")
        on_complete = self._callbacks.get("complete")
        on_error = self._callbacks.get("error")

        preset = QUALITY_PRESETS.get(quality, QUALITY_PRESETS["best_mp4"])
        format_str = preset["format"]
        merge_ext = preset.get("ext", "mp4")

        cookie_args = _build_cookie_args()

        cmd = [
            "yt-dlp",
            "--format", format_str,
            "--output", os.path.join(self.save_dir, "%(title)s.%(ext)s"),
            "--no-playlist",
            "--no-warnings",
            "--newline",
            "--no-colors",
            "--no-overwrites",
            "--continue",              # Resume partial download
            "--socket-timeout", "15",
            "--retries", "5",
            "--fragment-retries", "5",
        ] + cookie_args

        # Merge format
        if preset.get("postprocess"):
            cmd.extend(preset["postprocess"])
        elif merge_ext in ("mp4", "mkv"):
            cmd.extend(["--merge-output-format", merge_ext])

        # Thumbnail embed
        if merge_ext == "mp4":
            cmd.append("--embed-thumbnail")

        # Metadata
        cmd.append("--embed-metadata")

        # Subtitles
        if subtitle_lang:
            cmd.extend([
                "--write-sub",
                "--sub-lang", subtitle_lang,
                "--embed-subs",
            ])

        cmd.append(url)

        print("[FastDM] yt-dlp cmd: {}".format(" ".join(cmd[:10]) + "..."),
              file=sys.stderr)

        try:
            self._process = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                preexec_fn=os.setsid,
            )

            filename = ""
            last_update = 0

            # Regex patterns untuk yt-dlp output
            re_dest = re.compile(r'\[download\]\s+Destination:\s+(.+)')
            re_progress = re.compile(
                r'\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)\s+ETA\s+(\S+)'
            )
            re_progress2 = re.compile(
                r'\[download\]\s+(\d+\.?\d*)%\s+of\s+~?\s*(\S+)\s+at\s+(\S+)'
            )
            re_progress3 = re.compile(
                r'\[download\]\s+(\d+\.?\d*)%'
            )
            re_already = re.compile(
                r'\[download\].*has already been downloaded'
            )
            re_merge = re.compile(
                r'\[Merger\]|\[ffmpeg\]|Merging'
            )

            for line in self._process.stdout:
                if self._cancelled or self._paused:
                    self._kill()
                    if self._cancelled and on_error:
                        on_error("Cancelled")
                    elif self._paused and on_progress:
                        on_progress({
                            "percent": -1,
                            "speed": "",
                            "eta": "",
                            "filename": filename or "YouTube video",
                            "status": "paused",
                        })
                    return

                line = line.strip()
                if not line:
                    continue

                # Destination
                m = re_dest.search(line)
                if m:
                    filename = os.path.basename(m.group(1).strip())
                    continue

                # Already downloaded
                if re_already.search(line):
                    if on_complete:
                        on_complete({
                            "filename": filename,
                            "status": "completed",
                            "message": "Already downloaded",
                        })
                    return

                # Merging
                if re_merge.search(line):
                    if on_progress:
                        on_progress({
                            "percent": 99.0,
                            "speed": "",
                            "eta": "merging...",
                            "total_size": "",
                            "filename": filename or "YouTube video",
                            "status": "merging",
                        })
                    continue

                # Progress (try multiple patterns)
                m = re_progress.search(line)
                if not m:
                    m = re_progress2.search(line)
                if not m:
                    m = re_progress3.search(line)

                if m and on_progress:
                    now = time.monotonic()
                    if (now - last_update) >= 0.25:
                        groups = m.groups()
                        on_progress({
                            "percent": float(groups[0]),
                            "total_size": groups[1] if len(groups) > 1 else "",
                            "speed": groups[2] if len(groups) > 2 else "",
                            "eta": groups[3] if len(groups) > 3 else "",
                            "filename": filename or "YouTube video",
                            "status": "downloading",
                        })
                        last_update = now

            returncode = self._process.wait()
            self._process = None

            if self._cancelled:
                if on_error:
                    on_error("Cancelled")
                return

            if self._paused:
                return

            if returncode == 0:
                if on_complete:
                    on_complete({
                        "filename": filename,
                        "status": "completed",
                    })
            else:
                if on_error:
                    on_error("yt-dlp exit code: {}".format(returncode))

        except FileNotFoundError:
            if on_error:
                on_error("yt-dlp not found.\nInstall: sudo apt install yt-dlp")
        except Exception as e:
            if on_error:
                on_error(str(e))
