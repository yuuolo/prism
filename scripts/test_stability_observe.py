"""Isolated loopback tests; no host controller or network settings are used."""
import contextlib
import io
import json
import os
import tempfile
import threading
import unittest
from argparse import Namespace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

import stability_observe as observer


class ObserveTests(unittest.TestCase):
    def test_projection_and_origin_validation(self):
        for value in ([], None, {}, {"schema": 2}):
            with self.assertRaises(ValueError):
                observer.project(value)
        result = observer.project({"schema": 1, "rx_bytes": 5, "tx_bytes": True,
            "secret": "private", "config": {"password": "private"},
            "fault": {"stage": "adapter_rx", "reason": "task_exited", "packet": "private"}})
        self.assertEqual(result, {"rx_bytes": 5, "enabled": False, "version": "",
            "fault": {"stage": "adapter_rx", "reason": "task_exited"}})
        for origin in ("https://127.0.0.1:80", "http://example.com:80", "http://1.1.1.1:80",
                       "http://user:secret@127.0.0.1:80", "http://127.0.0.1:80/config"):
            with self.assertRaises(ValueError):
                observer.endpoint(origin)

    def test_authenticated_collection_and_redirect_refusal(self):
        seen = []
        class Handler(BaseHTTPRequestHandler):
            redirect = False
            def log_message(self, *_):
                pass
            def do_GET(self):
                seen.append((self.path, self.headers.get("Authorization")))
                if self.redirect:
                    self.send_response(302)
                    self.send_header("Location", "/must-not-follow")
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(json.dumps({"schema": 1, "process_id": 7,
                        "rx_packets": 0, "secret": "must-not-export"}).encode())
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"PRISM_CONTROLLER_SECRET": "test-secret"}), contextlib.redirect_stdout(io.StringIO()):
                args = Namespace(controller=f"http://127.0.0.1:{server.server_port}",
                    seconds=0.05, interval=0.1, output=Path(directory) / "health.jsonl")
                summary = observer.observe(args)
                self.assertEqual(summary["errors"], 0)
                data = args.output.read_text(encoding="utf-8")
                self.assertNotIn("secret", data)
                rows = [json.loads(line) for line in data.splitlines()]
                self.assertGreaterEqual(len(rows), 1)
                self.assertTrue(all(row["health"]["rx_packets"] == 0 for row in rows))
                self.assertEqual(seen, [("/prism/health", "Bearer test-secret")] * len(rows))
                with self.assertRaises(FileExistsError):
                    observer.observe(args)
                Handler.redirect = True
                args.output = Path(directory) / "redirect.jsonl"
                summary = observer.observe(args)
                self.assertGreaterEqual(summary["errors"], 1)
                self.assertEqual(seen[-1][0], "/prism/health")
                self.assertEqual(len(seen), len(rows) + summary["errors"])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
