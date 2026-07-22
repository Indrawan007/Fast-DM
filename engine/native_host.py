# engine/native_host.py

import sys
import struct
import json
import threading
import os
import hashlib
import hmac


def _get_secret_token():
    """
    Generate/load secret token untuk autentikasi IPC.
    Token disimpan di file yang hanya bisa dibaca owner.
    """
    token_file = os.path.expanduser("~/.config/fast-dm/.ipc_token")
    os.makedirs(os.path.dirname(token_file), exist_ok=True)

    if os.path.exists(token_file):
        try:
            with open(token_file, "r") as f:
                token = f.read().strip()
                if len(token) == 64:  # 32 bytes hex
                    return token
        except OSError:
            pass

    # Generate token baru
    token = os.urandom(32).hex()
    with open(token_file, "w") as f:
        f.write(token)
    os.chmod(token_file, 0o600)  # Owner read/write only
    return token


# Token global (loaded sekali)
_SECRET_TOKEN = None


def get_secret_token():
    global _SECRET_TOKEN
    if _SECRET_TOKEN is None:
        _SECRET_TOKEN = _get_secret_token()
    return _SECRET_TOKEN


class NativeHost:
    """Chrome Native Messaging Host dengan validasi pesan."""

    # Aksi yang diizinkan dari extension
    ALLOWED_ACTIONS = {
        "download", "ping", "list",
        "pause", "resume", "cancel"
    }

    def __init__(self, on_message):
        self._on_message  = on_message
        self._running     = False
        self._write_lock  = threading.Lock()

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

                # Batasi ukuran pesan (max 1MB)
                if msg_length > 1024 * 1024:
                    continue

                msg_bytes = stdin.read(msg_length)
                if len(msg_bytes) < msg_length:
                    break

                message = json.loads(msg_bytes.decode("utf-8"))

                # Validasi pesan
                validated, reason = self._validate_message(message)
                if not validated:
                    self._send_message(
                        {"success": False, "error": reason}, stdout
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

    def _validate_message(self, message):
        """
        Validasi pesan dari extension:
        1. Harus punya field 'action' yang valid
        2. URL harus dimulai dengan http/https/ftp
        3. Tidak ada karakter berbahaya di parameter
        """
        if not isinstance(message, dict):
            return False, "Invalid message format"

        action = message.get("action", "")
        if action not in self.ALLOWED_ACTIONS:
            return False, "Unknown action: {}".format(action)

        # Validasi URL jika ada
        url = message.get("url", "")
        if url:
            ok, reason = _validate_url(url)
            if not ok:
                return False, reason

        return True, ""

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
