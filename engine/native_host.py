# engine/native_host.py

import sys
import struct
import json
import threading
import os
import subprocess


def _get_nmh_paths():
    """Semua lokasi file native messaging host manifest."""
    home = os.path.expanduser("~")
    return [
        # System-wide
        "/etc/opt/chrome/native-messaging-hosts/com.fastdm.native.json",
        "/etc/chromium/native-messaging-hosts/com.fastdm.native.json",
        # Per-user
        os.path.join(home, ".config/google-chrome/NativeMessagingHosts/com.fastdm.native.json"),
        os.path.join(home, ".config/chromium/NativeMessagingHosts/com.fastdm.native.json"),
    ]


def register_extension_id(ext_id):
    """
    Update semua NMH manifest dengan Extension ID.

    Dipanggil otomatis saat extension mengirim action "register".
    Tidak perlu lagi jalankan fast-dm-set-id manual.
    """
    if not ext_id or not isinstance(ext_id, str):
        return False, "Invalid extension ID"

    # Validasi format: hanya huruf kecil dan angka, 32 karakter
    import re
    if not re.match(r'^[a-z]{32}$', ext_id):
        # Beberapa ID bisa lebih panjang atau ada angka
        if not re.match(r'^[a-z0-9]{20,40}$', ext_id):
            return False, "Extension ID format invalid"

    origin = "chrome-extension://{}/".format(ext_id)
    updated = 0
    errors = []

    for manifest_path in _get_nmh_paths():
        if not os.path.exists(manifest_path):
            continue

        try:
            with open(manifest_path, 'r') as f:
                data = json.load(f)

            current_origins = data.get("allowed_origins", [])

            # Sudah terdaftar?
            if origin in current_origins:
                updated += 1
                continue

            # Update
            data["allowed_origins"] = [origin]

            # Cek apakah perlu sudo (system-wide paths)
            if manifest_path.startswith("/etc/"):
                # Tulis via sudo
                new_content = json.dumps(data, indent=2)
                result = subprocess.run(
                    ["sudo", "tee", manifest_path],
                    input=new_content,
                    capture_output=True,
                    text=True,
                    timeout=10
                )
                if result.returncode == 0:
                    updated += 1
                else:
                    # Coba tanpa sudo (jika user punya write permission)
                    try:
                        with open(manifest_path, 'w') as f:
                            json.dump(data, f, indent=2)
                        updated += 1
                    except PermissionError:
                        errors.append("Need sudo for: {}".format(manifest_path))
            else:
                # Per-user path — tulis langsung
                os.makedirs(os.path.dirname(manifest_path), exist_ok=True)
                with open(manifest_path, 'w') as f:
                    json.dump(data, f, indent=2)
                updated += 1

        except Exception as e:
            errors.append("{}: {}".format(manifest_path, str(e)))

    # Jika system-wide gagal, pastikan per-user manifest ada
    if updated == 0:
        home = os.path.expanduser("~")
        user_paths = [
            os.path.join(home, ".config/google-chrome/NativeMessagingHosts"),
            os.path.join(home, ".config/chromium/NativeMessagingHosts"),
        ]

        for dir_path in user_paths:
            try:
                os.makedirs(dir_path, exist_ok=True)
                manifest_path = os.path.join(
                    dir_path, "com.fastdm.native.json"
                )

                # Tentukan path binary
                native_path = "/opt/fast-dm/fast-dm-native"
                if not os.path.exists(native_path):
                    # Fallback: cari di lokasi development
                    script_dir = os.path.dirname(os.path.dirname(
                        os.path.abspath(__file__)))
                    native_path = os.path.join(
                        script_dir, "native_host_entry.sh"
                    )
                    if not os.path.exists(native_path):
                        native_path = os.path.join(
                            script_dir, "fast-dm-native"
                        )

                data = {
                    "name": "com.fastdm.native",
                    "description": "Fast Download Manager Native Host",
                    "path": native_path,
                    "type": "stdio",
                    "allowed_origins": [origin]
                }

                with open(manifest_path, 'w') as f:
                    json.dump(data, f, indent=2)
                updated += 1

            except Exception as e:
                errors.append(str(e))

    if updated > 0:
        print("[FastDM] Extension ID registered: {} ({} manifests)".format(
            ext_id, updated), file=sys.stderr)
        return True, "Registered in {} manifests".format(updated)
    else:
        err_msg = "; ".join(errors) if errors else "No manifest files found"
        print("[FastDM] Registration failed: {}".format(err_msg),
              file=sys.stderr)
        return False, err_msg


class NativeHost:
    """Chrome Native Messaging Host."""

    ALLOWED_ACTIONS = {
        "download", "ping", "list",
        "pause", "resume", "cancel",
        "register",
    }

    def __init__(self, on_message):
        self._on_message = on_message
        self._running    = False
        self._write_lock = threading.Lock()

    def run(self):
        self._running = True
        stdin  = sys.stdin.buffer
        stdout = sys.stdout.buffer

        while self._running:
            try:
                length_bytes = stdin.read(4)
                if not length_bytes or len(length_bytes) < 4:
                    break

                msg_length = struct.unpack("=I", length_bytes)[0]
                if msg_length > 1024 * 1024:
                    continue

                msg_bytes = stdin.read(msg_length)
                if len(msg_bytes) < msg_length:
                    break

                message = json.loads(msg_bytes.decode("utf-8"))

                # Validasi action
                action = message.get("action", "")
                if action not in self.ALLOWED_ACTIONS:
                    self._send_message(
                        {"success": False, "error": "Unknown action"},
                        stdout
                    )
                    continue

                # Handle register langsung di sini
                if action == "register":
                    ext_id = message.get("extension_id", "")
                    ok, msg = register_extension_id(ext_id)
                    self._send_message(
                        {"success": ok, "message": msg}, stdout
                    )
                    continue

                response = self._on_message(message)
                if response:
                    self._send_message(response, stdout)

            except (IOError, json.JSONDecodeError, struct.error):
                break
            except Exception as exc:
                self._send_message(
                    {"error": str(exc), "success": False}, stdout
                )

    def _send_message(self, message, stdout=None):
        if stdout is None:
            stdout = sys.stdout.buffer
        with self._write_lock:
            try:
                msg_bytes = json.dumps(message).encode("utf-8")
                stdout.write(struct.pack("=I", len(msg_bytes)))
                stdout.write(msg_bytes)
                stdout.flush()
            except IOError:
                pass

    def stop(self):
        self._running = False
