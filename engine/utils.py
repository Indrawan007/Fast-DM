# engine/utils.py

import re
import os
import time
import subprocess
from pathlib import Path


def check_aria2():
    """Cek apakah aria2c terinstall."""
    try:
        result = subprocess.run(
            ["aria2c", "--version"],
            capture_output=True,
            text=True,
            timeout=5
        )
        return result.returncode == 0
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def install_aria2():
    """Install aria2 otomatis."""
    try:
        subprocess.run(
            ["sudo", "apt-get", "install", "-y", "aria2"],
            check=True,
            timeout=120
        )
        return True
    except subprocess.CalledProcessError:
        return False


def sanitize_filename(filename):
    """Bersihkan nama file dari karakter berbahaya."""
    filename = re.sub(r'[<>:"/\\|?*\x00-\x1f]', '_', filename)
    name, ext = os.path.splitext(filename)
    if len(name) > 200:
        name = name[:200]
    return "{}{}".format(name, ext) if ext else name


def format_size(size_bytes):
    """Format bytes ke human readable."""
    if size_bytes <= 0:
        return "0 B"
    units = ["B", "KB", "MB", "GB", "TB"]
    i = 0
    size = float(size_bytes)
    while size >= 1024.0 and i < len(units) - 1:
        size /= 1024.0
        i += 1
    return "{:.1f} {}".format(size, units[i])


def format_speed(bytes_per_sec):
    """Format kecepatan download."""
    if bytes_per_sec <= 0:
        return "0 B/s"
    return "{}/s".format(format_size(bytes_per_sec))


def format_eta(seconds):
    """Format estimasi waktu tersisa."""
    if seconds <= 0:
        return "--"
    if seconds < 60:
        return "{}s".format(seconds)
    if seconds < 3600:
        m, s = divmod(seconds, 60)
        return "{}m {}s".format(m, s)
    h, remainder = divmod(seconds, 3600)
    m, s = divmod(remainder, 60)
    return "{}h {}m".format(h, m)


def extract_filename_from_url(url):
    """Ekstrak nama file dari URL."""
    from urllib.parse import urlparse, unquote
    try:
        parsed = urlparse(url)
        path = unquote(parsed.path)
        filename = os.path.basename(path)
        if not filename or '.' not in filename:
            filename = "download_{}".format(int(time.time()))
        return sanitize_filename(filename)
    except Exception:
        return "download_{}".format(int(time.time()))


def extract_filename_from_headers(headers):
    """Ekstrak nama file dari Content-Disposition header."""
    cd = headers.get("content-disposition", "")
    if not cd:
        return None

    match = re.search(r"filename\*=(?:UTF-8''|utf-8'')(.+?)(?:;|$)", cd, re.I)
    if match:
        from urllib.parse import unquote
        return sanitize_filename(unquote(match.group(1).strip()))

    match = re.search(r'filename="?([^";\n]+)"?', cd, re.I)
    if match:
        return sanitize_filename(match.group(1).strip())

    return None


def is_video_url(url, content_type=""):
    """Deteksi apakah URL adalah video."""
    # Import di sini untuk avoid circular import
    from engine.config import Config
    cfg = Config()

    video_types = [
        "video/",
        "application/x-mpegurl",
        "application/vnd.apple.mpegurl"
    ]
    for vt in video_types:
        if content_type.startswith(vt):
            return True

    from urllib.parse import urlparse
    try:
        path = urlparse(url).path.lower()
        for ext in cfg.video_extensions:
            if path.endswith(ext):
                return True
    except Exception:
        pass

    return False
