# engine/downloader.py

import json
import subprocess
import threading
import time
import os
import re
import signal
from dataclasses import dataclass, field
from enum import Enum
from typing import Callable, Optional
from pathlib import Path

from engine.config import Config
from engine.utils import (
    extract_filename_from_url,
    format_size,
    format_speed,
    format_eta,
    sanitize_filename,
)


class DownloadStatus(Enum):
    QUEUED      = "queued"
    DOWNLOADING = "downloading"
    PAUSED      = "paused"
    COMPLETED   = "completed"
    ERROR       = "error"
    CANCELLED   = "cancelled"


@dataclass
class DownloadItem:
    """Representasi satu download task."""
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

    # Internal — tidak di-compare / repr
    _process: object = field(default=None, repr=False, compare=False)
    _thread:  object = field(default=None, repr=False, compare=False)

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
        }


class DownloadEngine:
    """Engine download utama menggunakan aria2c sebagai backend."""

    def __init__(self):
        self.cfg = Config()
        self._downloads = {}          # id -> DownloadItem
        self._lock      = threading.Lock()
        self._counter   = 0
        self._on_update   = None
        self._on_complete = None

    # ------------------------------------------------------------------
    # Callbacks
    # ------------------------------------------------------------------

    def set_callbacks(self, on_update=None, on_complete=None):
        self._on_update   = on_update
        self._on_complete = on_complete

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def add_download(self, url, filename=None, save_dir=None,
                     headers=None, auto_start=True):
        """Tambah download baru. Returns download ID."""
        dl_id = self._generate_id()

        filename = sanitize_filename(filename) if filename \
                   else extract_filename_from_url(url)
        save_dir = save_dir or self.cfg.download_dir

        Path(save_dir).mkdir(parents=True, exist_ok=True)

        # Handle duplicate filename
        filepath = Path(save_dir) / filename
        if filepath.exists() and self.cfg.auto_file_renaming:
            base, ext = os.path.splitext(filename)
            counter = 1
            while filepath.exists():
                filename = "{}_{}{}".format(base, counter, ext)
                filepath = Path(save_dir) / filename
                counter += 1

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
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
            if item.status == DownloadStatus.DOWNLOADING:
                return

        item.status = DownloadStatus.DOWNLOADING
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
            if not item or item.status != DownloadStatus.DOWNLOADING:
                return
        item.status = DownloadStatus.PAUSED
        item.speed  = 0
        self._kill_process(item)

    def resume_download(self, dl_id):
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item or item.status != DownloadStatus.PAUSED:
                return
        self.start_download(dl_id)

    def cancel_download(self, dl_id):
        with self._lock:
            item = self._downloads.get(dl_id)
            if not item:
                return
        item.status = DownloadStatus.CANCELLED
        self._kill_process(item)

        partial   = Path(item.save_dir) / item.filename
        aria2ctrl = Path(item.save_dir) / "{}.aria2".format(item.filename)
        for f in (partial, aria2ctrl):
            try:
                f.unlink(missing_ok=True)
            except OSError:
                pass

    def remove_download(self, dl_id):
        self.cancel_download(dl_id)
        with self._lock:
            self._downloads.pop(dl_id, None)

    def get_download(self, dl_id):
        item = self._downloads.get(dl_id)
        return item.to_dict() if item else None

    def get_all_downloads(self):
        with self._lock:
            return [item.to_dict() for item in self._downloads.values()]

    def get_active_count(self):
        return sum(
            1 for item in self._downloads.values()
            if item.status == DownloadStatus.DOWNLOADING
        )

    def shutdown(self):
        with self._lock:
            items = list(self._downloads.values())
        for item in items:
            if item.status == DownloadStatus.DOWNLOADING:
                item.status = DownloadStatus.PAUSED
                self._kill_process(item)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _generate_id(self):
        self._counter += 1
        return "dl_{}_{}".format(int(time.time()), self._counter)

    def _build_aria2_cmd(self, item):
        """Build command aria2c yang dioptimasi."""
        cmd = [
            "aria2c",
            "--max-connection-per-server={}".format(self.cfg.max_connections),
            "--split={}".format(self.cfg.max_connections),
            "--min-split-size={}".format(self.cfg.min_split_size),
            "--piece-length={}".format(self.cfg.chunk_size),
            "--max-overall-download-limit={}".format(self.cfg.max_overall_speed),
            "--max-tries={}".format(self.cfg.retry_count),
            "--retry-wait={}".format(self.cfg.retry_wait),
            "--timeout={}".format(self.cfg.timeout),
            "--connect-timeout={}".format(self.cfg.timeout),
            "--disk-cache={}".format(self.cfg.disk_cache_size),
            "--file-allocation={}".format(self.cfg.file_allocation),
            "--enable-mmap=true",
            "--optimize-concurrent-downloads=true",
            "--stream-piece-selector=geom",
            "--dir={}".format(item.save_dir),
            "--out={}".format(item.filename),
            "--console-log-level=error",
            "--summary-interval=1",
            "--human-readable=false",
            "--show-console-readout=true",
            "--continue=true",
            "--auto-file-renaming=false",
            "--allow-overwrite=true",
            "--check-certificate=true",
            "--max-resume-failure-tries=5",
            "--uri-selector=adaptive",
        ]

        for key, value in item.headers.items():
            kl = key.lower()
            if kl == "user-agent":
                cmd.append("--user-agent={}".format(value))
            elif kl == "referer":
                cmd.append("--referer={}".format(value))
            elif kl == "cookie":
                cmd.append("--header=Cookie: {}".format(value))
            else:
                cmd.append("--header={}: {}".format(key, value))

        cmd.append(item.url)
        return cmd

    def _download_worker(self, dl_id):
        """Worker thread — jalankan aria2c dan parse output."""
        item = self._downloads.get(dl_id)
        if not item:
            return

        cmd = self._build_aria2_cmd(item)

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

            if returncode == 0:
                item.status   = DownloadStatus.COMPLETED
                item.progress = 100.0
                item.speed    = 0
                item.eta      = 0
            elif item.status in (DownloadStatus.CANCELLED,
                                  DownloadStatus.PAUSED):
                pass
            else:
                item.status    = DownloadStatus.ERROR
                item.error_msg = "aria2c exit code: {}".format(returncode)

        except FileNotFoundError:
            item.status    = DownloadStatus.ERROR
            item.error_msg = "aria2c not found. Run: sudo apt install aria2"
        except Exception as exc:
            item.status    = DownloadStatus.ERROR
            item.error_msg = str(exc)

        item._process = None

        if self._on_complete:
            self._on_complete(item.to_dict())
        if self._on_update:
            self._on_update(item.to_dict())

    def _parse_aria2_output(self, item, process):
        """Parse aria2c stdout untuk progress real-time."""
        # Pola utama: [#gid downloaded/total(pct%) CN:n DL:speed ETA:t]
        re_main  = re.compile(
            r'\[#\w+\s+(\d+)B/(\d+)B\((\d+)%\)\s+CN:(\d+)\s+DL:(\d+)B'
            r'(?:\s+ETA:(\S+))?'
        )
        re_prog  = re.compile(r'(\d+)B/(\d+)B\((\d+)%\)')
        re_speed = re.compile(r'DL:(\d+)B')
        re_cn    = re.compile(r'CN:(\d+)')
        re_eta   = re.compile(r'ETA:(\S+)')

        for line in process.stdout:
            if item.status in (DownloadStatus.CANCELLED,
                                DownloadStatus.PAUSED):
                break

            line = line.strip()
            if not line:
                continue

            m = re_main.search(line)
            if m:
                item.downloaded  = int(m.group(1))
                item.total_size  = int(m.group(2))
                item.progress    = float(m.group(3))
                item.connections = int(m.group(4))
                item.speed       = int(m.group(5))
                if m.group(6):
                    item.eta = self._parse_eta(m.group(6))
            else:
                mp = re_prog.search(line)
                if mp:
                    item.downloaded = int(mp.group(1))
                    item.total_size = int(mp.group(2))
                    item.progress   = float(mp.group(3))

                ms = re_speed.search(line)
                if ms:
                    item.speed = int(ms.group(1))

                mc = re_cn.search(line)
                if mc:
                    item.connections = int(mc.group(1))

                me = re_eta.search(line)
                if me:
                    item.eta = self._parse_eta(me.group(1))

            if self._on_update:
                self._on_update(item.to_dict())

    @staticmethod
    def _parse_eta(eta_str):
        """Parse ETA string ke detik. '1h2m3s' → 3723"""
        total = 0
        h = re.search(r'(\d+)h', eta_str)
        m = re.search(r'(\d+)m', eta_str)
        s = re.search(r'(\d+)s', eta_str)
        if h:
            total += int(h.group(1)) * 3600
        if m:
            total += int(m.group(1)) * 60
        if s:
            total += int(s.group(1))
        if not h and not m and not s:
            try:
                total = int(eta_str)
            except ValueError:
                pass
        return total

    def _kill_process(self, item):
        """Kill proses aria2c dengan bersih."""
        proc = item._process
        if proc and proc.poll() is None:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired,
                    OSError):
                try:
                    os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                except (ProcessLookupError, OSError):
                    pass
