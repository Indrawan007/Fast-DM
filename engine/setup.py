# engine/setup.py

import os
import json
import sys
import glob


HOST_NAME = "com.fastdm.native"
NATIVE_PATH = "/opt/fast-dm/fast-dm-native"

NATIVE_PATH_DEV = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "fast-dm-native"
)


def _get_extension_id():
    """
    Baca fixed Extension ID dari file.
    ID ini sama di semua browser karena manifest.json punya "key" field.
    """
    # Cari file EXTENSION_ID
    search_paths = [
        "/opt/fast-dm/EXTENSION_ID",
        os.path.join(os.path.dirname(os.path.dirname(
            os.path.abspath(__file__))), "EXTENSION_ID"),
    ]

    for path in search_paths:
        if os.path.exists(path):
            try:
                with open(path, "r") as f:
                    ext_id = f.read().strip()
                    if ext_id and len(ext_id) >= 20:
                        return ext_id
            except IOError:
                pass

    # Fallback: wildcard (kurang aman tapi tetap jalan)
    return None


def _get_native_path():
    """Tentukan path native host executable."""
    if os.path.exists(NATIVE_PATH):
        return NATIVE_PATH
    if os.path.exists(NATIVE_PATH_DEV):
        return NATIVE_PATH_DEV

    script_dir = os.path.dirname(os.path.dirname(
        os.path.abspath(__file__)))
    for name in ("fast-dm-native", "native_host_entry.sh"):
        candidate = os.path.join(script_dir, name)
        if os.path.exists(candidate):
            return candidate

    return NATIVE_PATH  # default


def _build_host_json(native_path, ext_id=None):
    """Build NMH manifest JSON."""
    if ext_id:
        origins = ["chrome-extension://{}/".format(ext_id)]
    else:
        origins = ["chrome-extension://*/*"]

    data = {
        "name": HOST_NAME,
        "description": "Fast Download Manager Native Host",
        "path": native_path,
        "type": "stdio",
        "allowed_origins": origins
    }
    return json.dumps(data, indent=2)


def get_all_nmh_dirs():
    """
    Dapatkan SEMUA direktori NativeMessagingHosts yang mungkin.

    Mencakup:
    - ~/.config/<browser>/NativeMessagingHosts   (standar)
    - ~/.local/share/ice/profiles/*/             (Linux Mint Ice/Helium WebApps)
    - ~/.local/share/*/profiles/*/               (profil Chromium lain)
    - Semua subfolder yang sudah ada NativeMessagingHosts
    """
    home = os.path.expanduser("~")
    config_dir = os.path.join(home, ".config")
    local_share = os.path.join(home, ".local", "share")
    dirs = set()

    # ── 1. Standard browser config dirs ──
    browser_configs = [
        "google-chrome",
        "chromium",
        "BraveSoftware/Brave-Browser",
        "vivaldi",
        "opera",
        "com.operasoftware.Opera",
        "microsoft-edge",
        "thorium",
        "ungoogled-chromium",
        "yandex-browser",
        "sidekick",
        "helium",
    ]

    for browser in browser_configs:
        dirs.add(os.path.join(config_dir, browser, "NativeMessagingHosts"))

    # ── 2. Linux Mint Ice / Helium / Peppermint WebApp profiles ──
    ice_dirs = [
        os.path.join(local_share, "ice", "profiles"),
        os.path.join(local_share, "helium", "profiles"),
    ]

    for ice_base in ice_dirs:
        if os.path.isdir(ice_base):
            try:
                for profile in os.listdir(ice_base):
                    profile_dir = os.path.join(ice_base, profile)
                    if os.path.isdir(profile_dir):
                        dirs.add(os.path.join(
                            profile_dir, "NativeMessagingHosts"
                        ))
            except OSError:
                pass

    # ── 3. Scan semua existing NativeMessagingHosts dirs ──
    scan_patterns = [
        os.path.join(config_dir, "*", "NativeMessagingHosts"),
        os.path.join(config_dir, "*", "*", "NativeMessagingHosts"),
        os.path.join(local_share, "*", "profiles", "*"),
    ]

    for pattern in scan_patterns:
        for found in glob.glob(pattern):
            if os.path.isdir(found):
                if found.endswith("NativeMessagingHosts"):
                    dirs.add(found)
                else:
                    # Ini adalah profile dir, tambah NMH subdir
                    dirs.add(os.path.join(found, "NativeMessagingHosts"))

    return sorted(dirs)


def check_and_setup():
    """
    Buat/update NMH manifest di semua lokasi browser.
    Dipanggil setiap kali fast-dm start.

    Returns: jumlah manifest yang dibuat/diupdate
    """
    native_path = _get_native_path()
    ext_id = _get_extension_id()
    host_json = _build_host_json(native_path, ext_id)

    created = 0
    all_dirs = get_all_nmh_dirs()

    for nmh_dir in all_dirs:
        manifest_path = os.path.join(nmh_dir, "{}.json".format(HOST_NAME))

        need_update = True
        if os.path.exists(manifest_path):
            try:
                with open(manifest_path, "r") as f:
                    existing = json.load(f)

                # Cek path dan origins masih benar
                path_ok = existing.get("path") == native_path

                if ext_id:
                    origin = "chrome-extension://{}/".format(ext_id)
                    origins_ok = origin in existing.get("allowed_origins", [])
                else:
                    origins_ok = True

                if path_ok and origins_ok:
                    need_update = False

            except (json.JSONDecodeError, IOError):
                pass

        if need_update:
            try:
                os.makedirs(nmh_dir, exist_ok=True)
                with open(manifest_path, "w") as f:
                    f.write(host_json)
                created += 1
            except PermissionError:
                pass
            except OSError:
                pass

    if created > 0:
        mode = "ID: {}".format(ext_id) if ext_id else "wildcard mode"
        print("[FastDM] Setup: {} manifest(s) updated ({})".format(
            created, mode), file=sys.stderr)

    return created


def get_detected_browsers():
    """Return list browser yang terdeteksi."""
    home = os.path.expanduser("~")
    config = os.path.join(home, ".config")
    found = []

    for browser in ["google-chrome", "chromium", "BraveSoftware",
                    "vivaldi", "opera", "microsoft-edge", "thorium",
                    "ungoogled-chromium", "helium"]:
        if os.path.isdir(os.path.join(config, browser)):
            found.append(browser.split("/")[0])

    # Ice profiles
    ice_dir = os.path.join(home, ".local", "share", "ice", "profiles")
    if os.path.isdir(ice_dir):
        try:
            profiles = [p for p in os.listdir(ice_dir)
                       if os.path.isdir(os.path.join(ice_dir, p))]
            if profiles:
                found.append("Ice/Helium ({} profiles)".format(len(profiles)))
        except OSError:
            pass

    return found
