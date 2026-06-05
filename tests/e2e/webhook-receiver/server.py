#!/usr/bin/env python3
"""Minimal HTTP target and webhook capture server for E2E tests."""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

_receipts: list[dict] = []
_lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def _send_json(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json(200, {"status": "ok"})
        elif self.path == "/echo":
            self._send_json(200, {"message": "ok"})
        elif self.path == "/receipts":
            with _lock:
                payload = {"count": len(_receipts), "receipts": list(_receipts)}
            self._send_json(200, payload)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length) if length else b""

        if self.path == "/webhook":
            with _lock:
                try:
                    _receipts.append(json.loads(body.decode() or "{}"))
                except json.JSONDecodeError:
                    _receipts.append({"raw": body.decode(errors="replace")})
            self._send_json(200, {"received": True})
        elif self.path == "/echo":
            self._send_json(200, {"message": "ok"})
        elif self.path == "/fail":
            self.send_response(500)
            self.end_headers()
            self.wfile.write(b"fail")
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, fmt: str, *args) -> None:
        return


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
