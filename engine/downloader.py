# engine/downloader.py

import json
import subprocess
import threading
import time
import os
import re
import signal
import tempfile
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from urllib.parse import urlparse, unquote
import sys

from engine.config import Config
from engine.utils import (
    extract_filename_from_url,
    resolve_filename,
    format_size,
    format_speed,
    format_eta,
    sanitize_filename,
    _content_type_to_ext,
)


class DownloadStatus(Enum):
    QUEUED      = "queued"
    RESOLVING   = "resolving"
    DOWNLOADING = "downloading"
    PAUSED      = "paused"
    COMPLETED   = "completed"
    ERROR       = "error"
    CANCELLED   = "cancelled"


@dataclass
class DownloadItem:
    id:          str
    url:         str
    filename:    str
    save_dir:    str
    status:      DownloadStatus = DownloadStatus.QUEUED
    total_size:  int   = 0
    downloaded:  int   = 0
    speed:       int   = 0
    eta:         int   = 0
    progress:    float = 0.0
    error_msg:   str   = ""
    connections: int   = 0
    created_at:  float = field(default_factory=time.time)
    headers:     dict  = field(default_factory=dict)
    final_url:   str   = ""
    retry_count: int   = 0

    _process:    object = field(default=None, repr=False, compare=False)
    _thread:     object = field(default=None, repr=False, compare=False)
    _input_file: object = field(default=None, repr=False, compare=False)
    _resolved:   bool  = field(default=False, repr=False, compare=False)

    def to_dict(self):
        return {
            "id":              self.id,
            "url":             self.url,
            "filename":        self.filename,
            "save_dir":        self.save_dir,
            "status":          self.status.value,
            "total_size":      self.total_size,
            "total_size_fmt":  format_size(self.total_size),
            "downloaded":      self.downloaded,
            "downloaded_fmt":  format_size(self.downloaded),
            "speed":           self.speed,
            "speed_fmt":       format_speed(self.speed),
            "eta":             self.eta,
            "eta_fmt":         format_eta(self.eta),
            "progress":        self.progress,
            "error_msg":       self.error_msg,
            "connections":     self.connections,
            "retry_count":     self.retry_count,
        }


class DownloadEngine:

    CHROME_UA = (
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
        "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
    )

    # Exit codes aria2c yang bisa di-retry
    RETRYABLE_CODES = {
        2,   # Timeout
        5,   # Too slow
        6,   # Network error
        7,   # Unfinished (partial)
        19,  # DNS failed
        22,  # HTTP error (bisa temporary)
        23,  # Redirect error
        24,  # Too many redirects
        28,  # Checksum error
        30,  # TLS error
    }

    # Exit codes yang TIDAK bisa di-retry (fatal)
    FATAL_CODES = {
        3,   # 404 Not found
        9,   # Disk full
        13,  # File exists (handled separately)
        25,  # Auth failed
    }

    def __init__(self):
        self.cfg = Config()
        self._downloads = {}
        self._lock      = threading.Lock()
        self._counter   = 0
        self._on_update   = None
        self._on_complete = None
        self._tmp_dir = os.path.join(tempfile.gettempdir(), "fast-dm")
        os.makedirs(self._tmp_dir, exist_ok=True)

    def set_callbacks(self, on_update=None, on_complete=None):
        self._on_update   = on_update
        self._on_complete = on_complete

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def add_download(self, url, filename=None, save_dir=None,
                     headers=None, auto_start=True):
        dl_id = self._generate_id()
        save_dir = save_dir or self.cfg.download_dir
        Path(save_dir).mkdir(parents=True, exist_ok=True)

        if filename:
            filename = sanitize_filename(filename)
        if not filename:
            filename = extract_filename_from_url(url)
        if not filename:
            filename = "download_{}".format(int(time.time()))

        item = DownloadItem(
            id=dl_id, url=url, filename=filename,
            save_dir=save_dir, headers=headers or {}
        )

        with self._lock:
            self._downloads[dl_id] = item

        if auto_start:
            self.start_download(dl_id)
        return dl_id

    def start_download(self, dl_id):
        """Start atau restart download."""
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
            if item.status == DownloadStatus.DOWNLOADING:
                return

        # Reset error state
        item.error_msg = ""
        item.speed = 0
        item.status = DownloadStatus.RESOLVING
        if self._on_update:
            self._on_update(item.to_dict())

        t = threading.Thread(
            target=self._download_worker,
            args=(dl_id,),
            daemon=True,
            name="dl-{}".format(dl_id)
        )
        item._thread = t
        t.start()

    def pause_download(self, dl_id):
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
            if item.status not in (DownloadStatus.DOWNLOADING,
                                    DownloadStatus.RESOLVING):
                return
        item.status = DownloadStatus.PAUSED
        item.speed = 0
        self._kill_process(item)

    def resume_download(self, dl_id):
        """Resume download dari PAUSED atau ERROR state."""
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
            if item.status not in (DownloadStatus.PAUSED,
                                    DownloadStatus.ERROR):
                return
        self.start_download(dl_id)

    def retry_download(self, dl_id):
        """Retry download yang error — sama dengan resume."""
        self.resume_download(dl_id)

    def cancel_download(self, dl_id):
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
        item.status = DownloadStatus.CANCELLED
        self._kill_process(item)
        self._cleanup(item, remove_partial=True)

    def remove_download(self, dl_id):
        self.cancel_download(dl_id)
        with self._lock:
            self._downloads.pop(dl_id, None)

    def get_download(self, dl_id):
        item = self._downloads.get(dl_id)
        return item.to_dict() if item else None

    def get_all_downloads(self):
        with self._lock:
            return [it.to_dict() for it in self._downloads.values()]

    def get_active_count(self):
        return sum(
            1 for it in self._downloads.values()
            if it.status in (DownloadStatus.DOWNLOADING,
                              DownloadStatus.RESOLVING)
        )

    def shutdown(self):
        with self._lock:
            items = list(self._downloads.values())
        for item in items:
            if item.status in (DownloadStatus.DOWNLOADING,
                                DownloadStatus.RESOLVING):
                item.status = DownloadStatus.PAUSED
                self._kill_process(item)

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _generate_id(self):
        self._counter += 1
        return "dl_{}_{}".format(int(time.time()), self._counter)

    def _cleanup(self, item, remove_partial=False):
        """Cleanup temp files. Partial file hanya dihapus jika cancel."""
        if item._input_file:
            try:
                os.unlink(item._input_file)
            except OSError:
                pass
            item._input_file = None

        if remove_partial:
            for suffix in ("", ".aria2"):
                p = Path(item.save_dir) / "{}{}".format(item.filename, suffix)
                try:
                    p.unlink(missing_ok=True)
                except OSError:
                    pass

    def _resolve_filename(self, item):
        """Resolve nama file — skip jika sudah pernah resolve."""
        if item._resolved:
            return

        try:
            resolved = resolve_filename(
                item.url, item.headers, self.CHROME_UA
            )
            if resolved:
                fn = resolved.get("filename", "")
                if fn and fn != item.filename:
                    print("[FastDM] Resolved: {} -> {}".format(
                        item.filename, fn), file=sys.stderr)
                    item.filename = fn

                if resolved.get("final_url"):
                    item.final_url = resolved["final_url"]

                cl = resolved.get("content_length", 0)
                if cl and cl > 0:
                    item.total_size = cl

                ct = resolved.get("content_type", "")
                if ct and '.' not in item.filename:
                    ext = _content_type_to_ext(ct)
                    if ext:
                        item.filename = "{}{}".format(item.filename, ext)

        except Exception as e:
            print("[FastDM] Resolve warning: {}".format(e), file=sys.stderr)

        item._resolved = True

        # Handle duplicate
        filepath = Path(item.save_dir) / item.filename
        if filepath.exists() and self.cfg.auto_file_renaming:
            # Jangan rename jika ini adalah resume (ada .aria2 file)
            aria2_file = Path(item.save_dir) / "{}.aria2".format(item.filename)
            if not aria2_file.exists():
                base, ext = os.path.splitext(item.filename)
                counter = 1
                while filepath.exists():
                    item.filename = "{}_{}{}".format(base, counter, ext)
                    filepath = Path(item.save_dir) / item.filename
                    counter += 1

    def _create_input_file(self, item):
        """Buat aria2c input file."""
        download_url = item.final_url if item.final_url else item.url
        input_path = os.path.join(self._tmp_dir, "{}.txt".format(item.id))

        lines = [download_url]
        lines.append("  dir={}".format(item.save_dir))
        lines.append("  out={}".format(item.filename))
        lines.append("  continue=true")
        lines.append("  allow-overwrite=true")
        lines.append("  auto-file-renaming=false")

        for key, value in item.headers.items():
            kl = key.lower()
            if kl == "user-agent":
                pass
            elif kl == "referer":
                lines.append("  referer={}".format(value))
            elif kl == "cookie":
                lines.append("  header=Cookie: {}".format(value))
            else:
                lines.append("  header={}: {}".format(key, value))

        with open(input_path, 'w', encoding='utf-8') as f:
            f.write('\n'.join(lines) + '\n')

        item._input_file = input_path
        return input_path

    def _build_aria2_cmd(self, item):
        max_conn = self.cfg.max_connections
        input_file = self._create_input_file(item)

        cmd = [
            "aria2c",
            "--input-file={}".format(input_file),
            "--max-connection-per-server={}".format(max_conn),
            "--split={}".format(max_conn),
            "--min-split-size=1M",
            "--piece-length=1M",
            "--timeout=30",
            "--connect-timeout=15",
            "--lowest-speed-limit=1K",
            "--max-tries={}".format(self.cfg.retry_count),
            "--retry-wait={}".format(self.cfg.retry_wait),
            "--max-resume-failure-tries=5",
            "--disk-cache=64M",
            "--file-allocation={}".format(self.cfg.file_allocation),
            "--user-agent={}".format(self.CHROME_UA),
            "--console-log-level=notice",
            "--summary-interval=1",
            "--human-readable=false",
            "--show-console-readout=true",
            "--download-result=full",
            "--max-overall-download-limit={}".format(
                self.cfg.max_overall_speed
            ),
            "--check-integrity=false",
            "--check-certificate=false",
        ]

        return cmd

    def _download_worker(self, dl_id):
        item = self._downloads.get(dl_id)
        if not item:
            return

        max_auto_retry = self.cfg.retry_count
        attempt = 0

        while attempt <= max_auto_retry:
            if item.status in (DownloadStatus.CANCELLED,
                                DownloadStatus.PAUSED):
                return

            # ── Resolve filename (hanya sekali) ──
            if not item._resolved:
                item.status = DownloadStatus.RESOLVING
                if self._on_update:
                    self._on_update(item.to_dict())
                self._resolve_filename(item)

            if item.status in (DownloadStatus.CANCELLED,
                                DownloadStatus.PAUSED):
                return

            item.status = DownloadStatus.DOWNLOADING
            item.error_msg = ""
            if self._on_update:
                self._on_update(item.to_dict())

            # ── Run aria2c ──
            cmd = self._build_aria2_cmd(item)

            if attempt == 0:
                print("[FastDM] Downloading: {}".format(item.filename),
                      file=sys.stderr)
                print("[FastDM] Save to: {}/{}".format(
                    item.save_dir, item.filename), file=sys.stderr)
            else:
                print("[FastDM] Retry #{} for: {}".format(
                    attempt, item.filename), file=sys.stderr)
                item.retry_count = attempt

            returncode = self._run_aria2c(item, cmd)

            # ── Handle result ──
            if returncode is None:
                # Process was killed (pause/cancel)
                break

            if returncode == 0:
                # Sukses!
                self._check_actual_filename(item)
                item.status   = DownloadStatus.COMPLETED
                item.progress = 100.0
                item.speed    = 0
                item.eta      = 0
                break

            elif returncode == 13:
                # File exists
                check_file = Path(item.save_dir) / item.filename
                if check_file.exists() and check_file.stat().st_size > 0:
                    item.status   = DownloadStatus.COMPLETED
                    item.progress = 100.0
                    item.speed    = 0
                    item.eta      = 0
                    break
                else:
                    item.status    = DownloadStatus.ERROR
                    item.error_msg = "File conflict"
                    break

            elif returncode in self.RETRYABLE_CODES:
                # Error yang bisa di-retry otomatis
                attempt += 1
                if attempt <= max_auto_retry:
                    wait = min(attempt * 3, 15)
                    item.error_msg = "Retry #{} in {}s... (code {})".format(
                        attempt, wait, returncode)
                    item.speed = 0
                    if self._on_update:
                        self._on_update(item.to_dict())

                    # Tunggu sebelum retry
                    for _ in range(wait):
                        if item.status in (DownloadStatus.CANCELLED,
                                            DownloadStatus.PAUSED):
                            return
                        time.sleep(1)

                    # Cleanup temp input file sebelum retry
                    self._cleanup(item, remove_partial=False)
                    continue
                else:
                    # Max retry reached
                    item.status    = DownloadStatus.ERROR
                    item.error_msg = self._get_error_message(returncode)
                    break

            elif returncode in self.FATAL_CODES:
                # Fatal — tidak perlu retry
                item.status    = DownloadStatus.ERROR
                item.error_msg = self._get_error_message(returncode)
                break

            else:
                # Unknown error — coba retry sekali
                attempt += 1
                if attempt <= 1:
                    self._cleanup(item, remove_partial=False)
                    time.sleep(2)
                    continue
                item.status    = DownloadStatus.ERROR
                item.error_msg = self._get_error_message(returncode)
                break

        # ── Cleanup ──
        item._process = None
        self._cleanup(item, remove_partial=False)

        if self._on_complete:
            self._on_complete(item.to_dict())
        if self._on_update:
            self._on_update(item.to_dict())

    def _run_aria2c(self, item, cmd):
        """Jalankan aria2c dan return exit code, atau None jika killed."""
        try:
            process = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                preexec_fn=os.setsid,
            )
            item._process = process

            self._parse_aria2_output(item, process)

            returncode = process.wait()

            print("[FastDM] aria2c exit: {}".format(returncode),
                  file=sys.stderr)

            return returncode

        except FileNotFoundError:
            item.status    = DownloadStatus.ERROR
            item.error_msg = "aria2c not found. Run: sudo apt install aria2"
            return None

        except Exception as exc:
            item.status    = DownloadStatus.ERROR
            item.error_msg = str(exc)
            import traceback
            traceback.print_exc()
            return None

    def _get_error_message(self, code):
        """Human-readable error dari aria2c exit code."""
        messages = {
            1:  "Unknown error",
            2:  "Network timeout — check your connection",
            3:  "File not found on server (404)",
            4:  "Too many connections to server",
            5:  "Download too slow — server may be overloaded",
            6:  "Network error — check your connection",
            7:  "Download incomplete — will resume on retry",
            8:  "Server does not support resume",
            9:  "Not enough disk space",
            10: "Data mismatch",
            11: "Duplicate download",
            12: "Duplicate download",
            13: "File already exists",
            19: "DNS lookup failed — check your connection",
            22: "Server returned an error",
            23: "Too many redirects",
            24: "Too many redirects",
            25: "Login required (authentication failed)",
            28: "Data verification failed",
            30: "SSL/TLS connection failed",
        }
        return messages.get(code, "Error (exit code {})".format(code))

    def _check_actual_filename(self, item):
        """Cek nama file aktual setelah download selesai."""
        expected = Path(item.save_dir) / item.filename
        if expected.exists():
            return

        now = time.time()
        newest = None
        newest_time = 0

        try:
            for f in Path(item.save_dir).iterdir():
                if f.is_file() and not f.name.endswith('.aria2'):
                    mtime = f.stat().st_mtime
                    if (now - mtime) < 10 and mtime > newest_time:
                        newest = f
                        newest_time = mtime
        except OSError:
            return

        if newest and newest.name != item.filename:
            print("[FastDM] Actual filename: {}".format(newest.name),
                  file=sys.stderr)
            item.filename = newest.name

    def _parse_aria2_output(self, item, process):
        re_progress = re.compile(r'(\d+)B/(\d+)B\((\d+)%\)')
        re_speed    = re.compile(r'DL:(\d+)')
        re_cn       = re.compile(r'CN:(\d+)')
        re_eta      = re.compile(r'ETA:(\S+)')
        re_size     = re.compile(r'\[#\w+\s+(\d+)B/')
        re_saveas   = re.compile(
            r'(?:Saving|Download|File)\s.*?(?:as|to|named?)[:\s]+(.+)', re.I
        )

        last_update = 0.0
        update_interval = 0.2

        for line in process.stdout:
            if item.status in (DownloadStatus.CANCELLED,
                                DownloadStatus.PAUSED):
                break

            line = line.strip()
            if not line:
                continue

            if not line.startswith('[') and not line.startswith('***'):
                print("[FastDM] aria2c: {}".format(line), file=sys.stderr)

            m = re_saveas.search(line)
            if m:
                path = m.group(1).strip()
                basename = os.path.basename(path)
                cleaned = sanitize_filename(basename)
                if cleaned and '.' in cleaned:
                    if cleaned != item.filename:
                        print("[FastDM] aria2c filename: {}".format(cleaned),
                              file=sys.stderr)
                        item.filename = cleaned

            mp = re_progress.search(line)
            if mp:
                item.downloaded = int(mp.group(1))
                item.total_size = int(mp.group(2))
                item.progress   = float(mp.group(3))
            else:
                ms = re_size.search(line)
                if ms:
                    item.downloaded = int(ms.group(1))

            ms = re_speed.search(line)
            if ms:
                item.speed = int(ms.group(1))

            mc = re_cn.search(line)
            if mc:
                item.connections = int(mc.group(1))

            me = re_eta.search(line)
            if me:
                item.eta = self._parse_eta(me.group(1))

            if item.total_size > 0 and item.progress == 0 and item.downloaded > 0:
                item.progress = min(
                    99.9, (item.downloaded / item.total_size) * 100
                )

            now = time.monotonic()
            if self._on_update and (now - last_update) >= update_interval:
                self._on_update(item.to_dict())
                last_update = now

    @staticmethod
    def _parse_eta(eta_str):
        total = 0
        h = re.search(r'(\d+)h', eta_str)
        m = re.search(r'(\d+)m', eta_str)
        s = re.search(r'(\d+)s', eta_str)
        if h: total += int(h.group(1)) * 3600
        if m: total += int(m.group(1)) * 60
        if s: total += int(s.group(1))
        if not h and not m and not s:
            try: total = int(eta_str)
            except ValueError: pass
        return total

    def _kill_process(self, item):
        proc = item._process
        if proc and proc.poll() is None:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired, OSError):
                try:
                    os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                except (ProcessLookupError, OSError):
                    pass