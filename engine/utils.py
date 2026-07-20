# engine/utils.py

import re
import os
import time
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlparse, unquote, parse_qs


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


def sanitize_filename(filename):
    """Bersihkan nama file."""
    if not filename:
        return ""

    # Hapus query string & fragment
    filename = filename.split('?')[0].split('#')[0]

    # Decode URL encoding
    filename = unquote(filename)

    # Hapus karakter ilegal
    filename = re.sub(r'[<>:"/\\|?*\x00-\x1f]', '_', filename)

    # Hapus spasi/dot di awal dan akhir
    filename = filename.strip(' .')

    # Collapse multiple underscores
    filename = re.sub(r'_{2,}', '_', filename)

    # Batasi panjang
    name, ext = os.path.splitext(filename)
    if len(name) > 180:
        name = name[:180]

    result = "{}{}".format(name, ext) if ext else name

    if not result or result == '_':
        return ""

    return result


def format_size(size_bytes):
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
    if bytes_per_sec <= 0:
        return "0 B/s"
    return "{}/s".format(format_size(bytes_per_sec))


def format_eta(seconds):
    if seconds <= 0:
        return "--"
    if seconds < 60:
        return "{}s".format(seconds)
    if seconds < 3600:
        m, s = divmod(seconds, 60)
        return "{}m {}s".format(m, s)
    h, remainder = divmod(seconds, 3600)
    m, _ = divmod(remainder, 60)
    return "{}h {}m".format(h, m)


def extract_filename_from_url(url):
    """Ekstrak nama file dari URL path dan query params."""
    try:
        parsed = urlparse(url)

        # 1. Coba dari path
        path = unquote(parsed.path)
        basename = os.path.basename(path)
        cleaned = sanitize_filename(basename)
        if cleaned and '.' in cleaned and not _is_generic(cleaned):
            return cleaned

        # 2. Coba dari query parameter
        qs = parse_qs(parsed.query, keep_blank_values=False)
        for param in ('filename', 'file_name', 'fileName',
                      'name', 'title', 'fn', 'f', 'fname', 'file'):
            if param in qs:
                val = sanitize_filename(qs[param][0])
                if val and not _is_generic(val):
                    return val

        # 3. Fallback
        if cleaned and not _is_generic(cleaned):
            return cleaned

        return "download_{}".format(int(time.time()))

    except Exception:
        return "download_{}".format(int(time.time()))


def resolve_filename(url, headers=None, user_agent=None):
    """
    Resolve nama file ASLI dari server.

    Strategi:
    1. Coba curl GET dengan --range 0-0 (download 1 byte saja)
       Ini memaksa server mengirim Content-Disposition header
       yang sering TIDAK dikirim di HEAD request.
    2. Jika gagal, coba curl HEAD
    3. Jika masih gagal, coba aria2c --dry-run
    4. Fallback ke nama dari URL

    Kenapa GET range bukan HEAD?
    - CDN seperti PikPak, Google Drive, Mediafire HANYA kirim
      Content-Disposition di response GET, bukan HEAD
    - Range 0-0 hanya download 1 byte, sangat cepat
    """
    if not url:
        return None

    ua = user_agent or (
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
        "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
    )

    filename = None
    final_url = None
    content_length = 0
    content_type = ""

    # ══════════════════════════════════════════════════════════════
    # METODE 1: curl GET dengan Range header (paling akurat)
    #
    # Download 0 bytes tapi server tetap kirim semua headers
    # termasuk Content-Disposition yang berisi nama file asli
    # ══════════════════════════════════════════════════════════════

    filename, final_url, content_length, content_type = \
        _try_curl_get_range(url, headers, ua)

    if filename:
        print("[FastDM] Filename dari GET range: {}".format(filename),
              file=sys.stderr)
        return {
            "filename": filename,
            "final_url": final_url,
            "content_length": content_length,
            "content_type": content_type,
        }

    # ══════════════════════════════════════════════════════════════
    # METODE 2: curl HEAD (fallback)
    # ══════════════════════════════════════════════════════════════

    filename, final_url, content_length, content_type = \
        _try_curl_head(url, headers, ua)

    if filename:
        print("[FastDM] Filename dari HEAD: {}".format(filename),
              file=sys.stderr)
        return {
            "filename": filename,
            "final_url": final_url,
            "content_length": content_length,
            "content_type": content_type,
        }

    # ══════════════════════════════════════════════════════════════
    # METODE 3: aria2c --dry-run (terakhir)
    #
    # aria2c sendiri bisa resolve filename,
    # kita ambil dari outputnya tanpa download
    # ══════════════════════════════════════════════════════════════

    filename = _try_aria2c_dry_run(url, headers, ua)

    if filename:
        print("[FastDM] Filename dari aria2c dry-run: {}".format(filename),
              file=sys.stderr)
        return {
            "filename": filename,
            "final_url": None,
            "content_length": 0,
            "content_type": "",
        }

    # ══════════════════════════════════════════════════════════════
    # FALLBACK: dari URL
    # ══════════════════════════════════════════════════════════════

    fallback = extract_filename_from_url(url)
    print("[FastDM] Filename fallback dari URL: {}".format(fallback),
          file=sys.stderr)
    return {
        "filename": fallback,
        "final_url": None,
        "content_length": 0,
        "content_type": "",
    }


def _try_curl_get_range(url, headers, ua):
    """
    Curl GET dengan Range: bytes=0-0

    Server akan respond dengan:
    - 206 Partial Content + semua headers
    - Content-Disposition: attachment; filename="asli.mp4"
    - Content-Range: bytes 0-0/TOTALSIZE

    Ini hanya transfer 1 byte, tapi kita dapat nama file asli.
    """
    cmd = [
        "curl",
        "--silent",
        "--show-error",
        "--location",
        "--max-redirs", "10",
        "--max-time", "10",
        "--insecure",
        "--user-agent", ua,
        "--range", "0-0",           # Hanya 1 byte!
        "--dump-header", "-",       # Print headers ke stdout
        "--output", "/dev/null",    # Buang body
        "--write-out", "\n__URL__:%{url_effective}\n",
    ]

    if headers:
        for key, value in headers.items():
            if key.lower() != "user-agent":
                cmd.extend(["--header", "{}: {}".format(key, value)])

    cmd.append(url)

    return _run_curl_and_parse(cmd, url)


def _try_curl_head(url, headers, ua):
    """Curl HEAD request."""
    cmd = [
        "curl",
        "--head",
        "--silent",
        "--show-error",
        "--location",
        "--max-redirs", "10",
        "--max-time", "8",
        "--insecure",
        "--user-agent", ua,
        "--write-out", "\n__URL__:%{url_effective}\n",
    ]

    if headers:
        for key, value in headers.items():
            if key.lower() != "user-agent":
                cmd.extend(["--header", "{}: {}".format(key, value)])

    cmd.append(url)

    return _run_curl_and_parse(cmd, url)


def _run_curl_and_parse(cmd, original_url):
    """
    Jalankan curl dan parse response headers.
    Returns: (filename, final_url, content_length, content_type) or (None,...)
    """
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=12
        )

        if result.returncode != 0:
            return None, None, 0, ""

        output = result.stdout
        if not output:
            return None, None, 0, ""

        lines = output.strip().split('\n')

        content_disposition = ""
        content_length = 0
        content_type = ""
        content_range = ""
        final_url = None

        for line in lines:
            ls = line.strip()
            lower = ls.lower()

            if lower.startswith("content-disposition:"):
                content_disposition = ls.split(":", 1)[1].strip()

            elif lower.startswith("content-length:"):
                try:
                    cl = int(ls.split(":", 1)[1].strip())
                    # Hanya update jika lebih besar (range response = 1)
                    if cl > content_length:
                        content_length = cl
                except (ValueError, IndexError):
                    pass

            elif lower.startswith("content-range:"):
                # Format: bytes 0-0/TOTALSIZE
                content_range = ls.split(":", 1)[1].strip()

            elif lower.startswith("content-type:"):
                ct = ls.split(":", 1)[1].strip()
                # Ambil yang terakhir (setelah redirect)
                if ct and not ct.startswith("text/html"):
                    content_type = ct

            elif ls.startswith("__URL__:"):
                final_url = ls[len("__URL__:"):].strip()

        # Parse total size dari Content-Range
        if content_range:
            m = re.search(r'/(\d+)', content_range)
            if m:
                total = int(m.group(1))
                if total > content_length:
                    content_length = total

        # Parse filename
        filename = None

        if content_disposition:
            filename = _parse_content_disposition(content_disposition)

        if filename:
            filename = sanitize_filename(filename)
            if filename and not _is_generic(filename):
                return filename, final_url, content_length, content_type

        # Coba dari final URL
        if final_url and final_url != original_url:
            candidate = extract_filename_from_url(final_url)
            if candidate and '.' in candidate and not _is_generic(candidate):
                return candidate, final_url, content_length, content_type

        return None, final_url, content_length, content_type

    except (subprocess.TimeoutExpired, FileNotFoundError, Exception) as e:
        print("[FastDM] curl error: {}".format(e), file=sys.stderr)
        return None, None, 0, ""


def _try_aria2c_dry_run(url, headers, ua):
    """
    Gunakan aria2c --dry-run untuk resolve filename.

    aria2c --dry-run tidak download apa-apa tapi akan:
    1. Follow redirect
    2. Parse Content-Disposition
    3. Tentukan nama file
    4. Print nama file yang akan digunakan

    Kita parse dari output: "FILE: /path/to/filename.ext"
    """
    import tempfile

    # Buat input file untuk URL panjang
    tmp_dir = os.path.join(tempfile.gettempdir(), "fast-dm")
    os.makedirs(tmp_dir, exist_ok=True)
    input_path = os.path.join(tmp_dir, "dryrun_{}.txt".format(int(time.time())))

    try:
        with open(input_path, 'w') as f:
            f.write(url + '\n')

        cmd = [
            "aria2c",
            "--input-file={}".format(input_path),
            "--dry-run=true",
            "--user-agent={}".format(ua),
            "--console-log-level=info",
            "--check-certificate=false",
            "--max-tries=1",
            "--timeout=10",
            "--connect-timeout=8",
        ]

        if headers:
            for key, value in headers.items():
                kl = key.lower()
                if kl == "referer":
                    cmd.append("--referer={}".format(value))
                elif kl == "cookie":
                    cmd.append("--header=Cookie: {}".format(value))
                elif kl != "user-agent":
                    cmd.append("--header={}: {}".format(key, value))

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=12
        )

        output = result.stdout + "\n" + result.stderr

        # Cari pattern: "Saving the file as: FILENAME" atau
        # "File already exists. Renamed to FILENAME"
        # atau dari download result table

        # Pattern 1: "FILE:/path/to/file.ext"
        for line in output.split('\n'):
            line = line.strip()

            # Pattern: Saving file as
            m = re.search(r'(?:Saving|saved)\s+(?:the\s+)?(?:file\s+)?(?:as|to)[:\s]+(.+)',
                          line, re.I)
            if m:
                filepath = m.group(1).strip()
                basename = os.path.basename(filepath)
                cleaned = sanitize_filename(basename)
                if cleaned and '.' in cleaned and not _is_generic(cleaned):
                    return cleaned

            # Pattern: FILE:/home/user/Downloads/filename.ext
            m = re.search(r'\|FILE\|[^|]*\|(.+?)$', line)
            if m:
                filepath = m.group(1).strip()
                basename = os.path.basename(filepath)
                cleaned = sanitize_filename(basename)
                if cleaned and '.' in cleaned and not _is_generic(cleaned):
                    return cleaned

            # Pattern: [#xxxx] /path/to/file
            m = re.search(r'^\[#\w+\]\s+(.+)', line)
            if m:
                filepath = m.group(1).strip()
                if '/' in filepath:
                    basename = os.path.basename(filepath)
                    cleaned = sanitize_filename(basename)
                    if cleaned and '.' in cleaned and not _is_generic(cleaned):
                        return cleaned

        # Pattern dari download-result table
        # Cari baris dengan ekstensi file
        for line in output.split('\n'):
            m = re.search(r'(\S+\.(?:mp4|mkv|webm|avi|mov|flv|wmv|zip|rar|7z|tar|gz|iso|pdf|mp3|flac|m4a))\b',
                          line, re.I)
            if m:
                candidate = m.group(1)
                basename = os.path.basename(candidate)
                cleaned = sanitize_filename(basename)
                if cleaned and not _is_generic(cleaned):
                    return cleaned

    except (subprocess.TimeoutExpired, FileNotFoundError, Exception) as e:
        print("[FastDM] aria2c dry-run skip: {}".format(e), file=sys.stderr)
    finally:
        try:
            os.unlink(input_path)
        except OSError:
            pass

    return None


def _parse_content_disposition(cd):
    """Parse Content-Disposition header untuk nama file."""
    if not cd:
        return None

    # filename*=UTF-8''encoded (RFC 5987)
    match = re.search(
        r"filename\*\s*=\s*(?:[Uu][Tt][Ff]-8)?'[^']*'(.+?)(?:\s*;|$)", cd
    )
    if match:
        f = unquote(match.group(1).strip())
        if f:
            return f

    # filename="quoted"
    match = re.search(r'filename\s*=\s*"([^"]+)"', cd, re.I)
    if match:
        f = match.group(1).strip()
        if f:
            return f

    # filename=unquoted
    match = re.search(r'filename\s*=\s*([^\s;]+)', cd, re.I)
    if match:
        f = match.group(1).strip().strip('"\'')
        if f:
            return f

    return None


def _is_generic(filename):
    """Cek apakah filename terlalu generic."""
    if not filename:
        return True

    name_lower = os.path.splitext(filename)[0].lower()

    generic = {
        "download", "index", "file", "get", "fetch",
        "stream", "media", "content", "data", "output",
        "index.html", "index.htm", "default", "main",
        "video", "audio", "image", "document",
    }

    if name_lower in generic:
        return True
    if re.match(r'^download_\d+$', name_lower):
        return True
    # Pure angka (CDN biasanya pakai ID numerik di path)
    if name_lower.isdigit():
        return True
    # Terlalu pendek tanpa ekstensi
    if len(name_lower) <= 2 and '.' not in filename:
        return True

    return False


def _content_type_to_ext(content_type):
    """Convert MIME type ke file extension."""
    if not content_type:
        return ""
    ct = content_type.lower().split(';')[0].strip()
    mime_map = {
        "video/mp4":        ".mp4",
        "video/webm":       ".webm",
        "video/x-matroska": ".mkv",
        "video/quicktime":  ".mov",
        "video/x-msvideo":  ".avi",
        "video/x-flv":      ".flv",
        "video/3gpp":       ".3gp",
        "video/mp2t":       ".ts",
        "audio/mpeg":       ".mp3",
        "audio/mp4":        ".m4a",
        "audio/ogg":        ".ogg",
        "audio/wav":        ".wav",
        "audio/flac":       ".flac",
        "audio/webm":       ".weba",
        "application/pdf":  ".pdf",
        "application/zip":  ".zip",
        "application/gzip": ".gz",
        "application/x-rar-compressed": ".rar",
        "application/x-7z-compressed":  ".7z",
        "application/x-tar":            ".tar",
        "application/x-bzip2":          ".bz2",
        "application/x-iso9660-image":  ".iso",
        "application/octet-stream":     "",
        "image/jpeg":       ".jpg",
        "image/png":        ".png",
        "image/gif":        ".gif",
        "image/webp":       ".webp",
    }
    return mime_map.get(ct, "")


def is_video_url(url, content_type=""):
    from engine.config import Config
    cfg = Config()
    video_types = ["video/", "application/x-mpegurl",
                   "application/vnd.apple.mpegurl"]
    for vt in video_types:
        if content_type.startswith(vt):
            return True
    try:
        path = urlparse(url).path.lower()
        for ext in cfg.video_extensions:
            if path.endswith(ext):
                return True
    except Exception:
        pass
    return False