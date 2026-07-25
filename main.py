#!/usr/bin/env python3
# main.py

import sys
import os
import json
import signal

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from engine import DownloadEngine, NativeHost, Config
from engine.utils import check_aria2

_SOCKET_PATH = "/tmp/fast-dm-{}.sock".format(os.getuid())


def _handle_message(msg, engine, window=None):
    action = msg.get("action", "")

    if action == "register":
        ext_id = msg.get("extension_id", "")
        from engine.native_host import register_extension_id
        ok, message = register_extension_id(ext_id)
        return {"success": ok, "message": message}

    elif action == "download":
        url      = msg.get("url", "")
        filename = msg.get("filename")
        headers  = msg.get("headers", {})
        if not url:
            return {"success": False, "error": "No URL"}
        if window:
            dl_id = window.add_download_from_extension(
                url, filename=filename, headers=headers
            )
        else:
            dl_id = engine.add_download(
                url, filename=filename, headers=headers
            )
        return {"success": True, "id": dl_id}

    elif action == "ping":
        return {"success": True, "status": "running"}

    elif action == "list":
        return {"success": True, "downloads": engine.get_all_downloads()}

    elif action == "pause":
        engine.pause_download(msg.get("id", ""))
        return {"success": True}

    elif action == "resume":
        engine.resume_download(msg.get("id", ""))
        return {"success": True}

    elif action == "cancel":
        engine.cancel_download(msg.get("id", ""))
        return {"success": True}

    return {"success": False, "error": "Unknown action: {}".format(action)}


def _start_socket_server(engine, window):
    import socket
    import threading

    try:
        os.unlink(_SOCKET_PATH)
    except FileNotFoundError:
        pass

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(_SOCKET_PATH)
    os.chmod(_SOCKET_PATH, 0o600)
    server.listen(5)
    server.settimeout(1.0)

    def _listen():
        while True:
            try:
                conn, _ = server.accept()
            except socket.timeout:
                continue
            except OSError:
                break

            try:
                chunks = []
                conn.settimeout(5.0)
                while True:
                    chunk = conn.recv(4096)
                    if not chunk:
                        break
                    chunks.append(chunk)
                    if sum(len(c) for c in chunks) > 65536:
                        break
                conn.close()

                data = b"".join(chunks)
                if data:
                    try:
                        msg = json.loads(data.decode("utf-8"))
                    except json.JSONDecodeError:
                        continue
                    _handle_message(msg, engine, window)

            except Exception:
                pass

    t = threading.Thread(target=_listen, daemon=True, name="socket-server")
    t.start()


def run_gui():
    # ── Auto-setup NMH sebelum GUI muncul ──
    from engine.setup import check_and_setup
    check_and_setup()

    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk

    from gui.manager import ManagerWindow

    engine = DownloadEngine()
    window = ManagerWindow(engine)
    window.show_all()

    _start_socket_server(engine, window)

    Gtk.main()


def run_native_host():
    import socket
    import time

    # ── Auto-setup saat native host dipanggil juga ──
    from engine.setup import check_and_setup
    check_and_setup()

    def _forward(msg):
        action = msg.get("action", "")

        if action == "register":
            ext_id = msg.get("extension_id", "")
            from engine.native_host import register_extension_id
            ok, message = register_extension_id(ext_id)
            _try_forward_to_gui(msg)
            return {"success": ok, "message": message}

        return _try_forward_to_gui(msg)

    def _try_forward_to_gui(msg):
        try:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.settimeout(3)
            sock.connect(_SOCKET_PATH)
            sock.sendall(json.dumps(msg).encode("utf-8"))
            sock.shutdown(socket.SHUT_WR)
            sock.close()
            return {"success": True}

        except (ConnectionRefusedError, FileNotFoundError):
            import subprocess
            subprocess.Popen(
                [sys.executable, os.path.abspath(__file__)],
                start_new_session=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            time.sleep(2.0)

            try:
                sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                sock.settimeout(3)
                sock.connect(_SOCKET_PATH)
                sock.sendall(json.dumps(msg).encode("utf-8"))
                sock.shutdown(socket.SHUT_WR)
                sock.close()
                return {"success": True, "note": "GUI started"}
            except Exception as e:
                return {"success": False, "error": str(e)}

        except Exception as e:
            return {"success": False, "error": str(e)}

    host = NativeHost(_forward)
    host.run()


def main():
    if not check_aria2():
        print("=" * 50)
        print("ERROR: aria2c not found.")
        print("Install: sudo apt install aria2")
        print("=" * 50)
        sys.exit(1)

    signal.signal(signal.SIGINT, signal.SIG_DFL)

    if "--native" in sys.argv:
        run_native_host()
    else:
        run_gui()


if __name__ == "__main__":
    main()
