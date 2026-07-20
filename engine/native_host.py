# engine/native_host.py

import sys
import struct
import json
import threading


class NativeHost:
    """Chrome Native Messaging Host handler (stdio protocol)."""

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
                if msg_length > 1024 * 1024:
                    continue

                msg_bytes = stdin.read(msg_length)
                if len(msg_bytes) < msg_length:
                    break

                message  = json.loads(msg_bytes.decode("utf-8"))
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
