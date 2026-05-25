from __future__ import annotations

import json
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from .config import load_settings
from .core import GamblerService
from .db import Store, dumps
from .web import INDEX_HTML


settings = load_settings()
service = GamblerService(settings, Store(settings.database_url))


class Handler(BaseHTTPRequestHandler):
    server_version = "gambler-poc/0.1"

    def normalized_path(self) -> str:
        path = self.path.split("?", 1)[0]
        base_path = settings.base_path
        if base_path and path.startswith(base_path):
            stripped = path[len(base_path) :]
            return stripped or "/"
        return path

    def log_message(self, fmt: str, *args: Any) -> None:
        print(f"{self.address_string()} - {fmt % args}", flush=True)

    def read_json(self) -> dict[str, Any]:
        length = int(self.headers.get("Content-Length", "0") or 0)
        if not length:
            return {}
        return json.loads(self.rfile.read(length).decode("utf-8"))

    def send_json(self, payload: Any, status: HTTPStatus = HTTPStatus.OK) -> None:
        data = dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_html(self, body: str) -> None:
        rendered = body.replace("<body>", f'<body data-base-path="{settings.base_path}">')
        data = rendered.encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        try:
            path = self.normalized_path()
            if path in {"/", "/index.html"}:
                self.send_html(INDEX_HTML)
            elif path == "/healthz":
                self.send_json({"ok": True, "component": settings.component})
            elif path == "/readyz":
                self.send_json({"ok": True, "database": service.store.status()})
            elif path == "/api/status":
                self.send_json(service.status())
            elif path == "/api/snapshots/latest":
                self.send_json({"item": service.store.latest_snapshot()})
            elif path == "/api/candidates":
                self.send_json({"items": service.store.candidates(limit=50)})
            elif path == "/api/ledger":
                self.send_json({"items": service.store.simulated_bets(limit=50)})
            elif path == "/api/hermes":
                self.send_json(
                    {
                        "mode": "poc_view",
                        "summary": "Hermes integration is read-only in this POC. Reflections are loaded from Postgres when available.",
                        "reflections": service.store.hermes_reflections(limit=25),
                    }
                )
            else:
                self.send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)
        except Exception as exc:
            self.send_json({"error": str(exc)}, HTTPStatus.INTERNAL_SERVER_ERROR)

    def do_POST(self) -> None:
        try:
            path = self.normalized_path()
            if path == "/api/scan":
                payload = self.read_json()
                self.send_json(service.scan(include_live=bool(payload.get("include_live"))))
            elif path == "/api/simulate":
                payload = self.read_json()
                candidate_id = str(payload.get("candidate_id") or "")
                stake = float(payload.get("stake") or settings.default_stake)
                item = service.store.simulate_bet(candidate_id, stake)
                service.store.record_audit("paper_bet_created", {"candidate_id": candidate_id, "stake": stake})
                self.send_json({"item": item}, HTTPStatus.CREATED)
            else:
                self.send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)
        except ValueError as exc:
            self.send_json({"error": str(exc)}, HTTPStatus.BAD_REQUEST)
        except Exception as exc:
            self.send_json({"error": str(exc)}, HTTPStatus.INTERNAL_SERVER_ERROR)


def main() -> int:
    print(
        f"starting {settings.component} on {settings.host}:{settings.port} mode={settings.mode}",
        flush=True,
    )
    server = ThreadingHTTPServer((settings.host, settings.port), Handler)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
