# engine/config.py

import os
import json
from pathlib import Path


class Config:
    """Singleton konfigurasi."""

    _instance = None
    _CONFIG_DIR = Path.home() / ".config" / "fast-dm"
    _CONFIG_FILE = _CONFIG_DIR / "config.json"

    DEFAULTS = {
        "download_dir": str(Path.home() / "Downloads"),
        "max_connections": 16,        # Max koneksi per file (aria2 max 16)
        "max_concurrent": 3,          # File simultan
        "min_split_size": "1M",     # Lebih kecil = lebih banyak split = lebih cepat
        "chunk_size": "1M",           # Piece length
        "max_overall_speed": "0",     # 0 = unlimited
        "retry_count": 5,
        "retry_wait": 3,
        "timeout": 15,                # Lebih agresif (was 30)
        "continue_download": True,
        "auto_file_renaming": True,
        "disk_cache_size": "128M",    # Lebih besar (was 64M)
        "file_allocation": "falloc",  # fallocate (instan di ext4)
        "native_host_name": "com.fastdm.native",
        "log_level": "warn",
        "video_extensions": [
            ".mp4", ".mkv", ".webm", ".avi", ".mov",
            ".flv", ".wmv", ".m4v", ".3gp", ".ts",
            ".m3u8"
        ],
        "intercept_min_size": 1048576,
    }

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
            cls._instance._loaded = False
        return cls._instance

    def __init__(self):
        if not self._loaded:
            self._data = dict(self.DEFAULTS)
            self._load()
            self._loaded = True

    def _load(self):
        try:
            if self._CONFIG_FILE.exists():
                with open(self._CONFIG_FILE, "r") as f:
                    saved = json.load(f)
                self._data.update(saved)
        except (json.JSONDecodeError, IOError):
            pass

    def save(self):
        self._CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        diff = {}
        for k, v in self._data.items():
            if k not in self.DEFAULTS or self.DEFAULTS[k] != v:
                diff[k] = v
        with open(self._CONFIG_FILE, "w") as f:
            json.dump(diff, f, indent=2)

    def __getattr__(self, name):
        if name.startswith("_") or name in ("DEFAULTS", "save"):
            return super().__getattribute__(name)
        try:
            return self._data[name]
        except KeyError:
            raise AttributeError("Config has no '{}'".format(name))

    def __setattr__(self, name, value):
        if name.startswith("_") or name in ("DEFAULTS",):
            super().__setattr__(name, value)
        else:
            self._data[name] = value

    def get(self, key, default=None):
        return self._data.get(key, default)
