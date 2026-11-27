"""Exercise the packaged core on loopback without touching system proxy/TUN."""
import http.server
import json
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"prism-release-smoke-ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
api_port, mixed = port(), port()
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def api(path, method="GET", body=None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{api_port}{path}", data=data, method=method, headers={"Content-Type": "application/json", "Authorization": "Bearer smoke-local"})
    with opener.open(request, timeout=10) as response:
        return response.read()

def transfer():
    with socket.create_connection(("127.0.0.1", mixed), timeout=5) as sock:
        sock.sendall(f"GET http://127.0.0.1:{server.server_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{server.server_port}\r\nConnection: close\r\n\r\n".encode())
        response = b""
        while chunk := sock.recv(8192):
            response += chunk
        assert b"prism-release-smoke-ok" in response, response

with tempfile.TemporaryDirectory(prefix="prism-smoke-") as work:
    config = Path(work) / "config.yaml"
    config.write_text(f"proxies: []\nmixed-port: {mixed}\nexternal-controller: 127.0.0.1:{api_port}\nsecret: smoke-local\nmode: rule\nrules: ['MATCH,DIRECT']\n", encoding="utf-8")
    with (Path(work) / "core.log").open("w") as log:
        process = subprocess.Popen([str(ROOT / "target/release/prism-core.exe"), "-f", str(config), "-d", work], stdout=log, stderr=log, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        try:
            for attempt in range(100):
                try:
                    version = json.loads(api("/version"))
                    break
                except OSError:
                    if process.poll() is not None:
                        raise RuntimeError("release core exited during startup: " + (Path(work) / "core.log").read_text(errors="replace"))
                    time.sleep(0.1)
            else:
                raise RuntimeError("release core API startup timed out")
            transfer()
            for _ in range(20):
                api("/configs", "PUT", {})
                transfer()
            print(json.dumps({"version": version, "reload_cycles": 20, "http_proxy_transfers": 21, "result": "passed"}))
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
server.shutdown()
