from __future__ import annotations

import json
import uuid
from datetime import UTC, datetime
from typing import Any

try:
    import psycopg
    from psycopg.rows import dict_row
    from psycopg.types.json import Jsonb
except Exception:  # pragma: no cover - lets local no-dependency checks pass
    psycopg = None
    dict_row = None
    Jsonb = None


SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS odds_snapshots (
  id text PRIMARY KEY,
  observed_at timestamptz NOT NULL,
  source text NOT NULL,
  mode text NOT NULL,
  sport_keys text[] NOT NULL,
  event_count integer NOT NULL,
  payload jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS candidate_bets (
  id text PRIMARY KEY,
  snapshot_id text REFERENCES odds_snapshots(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  sport_key text NOT NULL,
  event_id text,
  event_name text,
  competition text,
  market_id text,
  market_name text,
  market_kind text,
  outcome_id text,
  outcome_name text,
  decimal_odds numeric,
  rationale jsonb NOT NULL,
  status text NOT NULL DEFAULT 'candidate'
);

CREATE TABLE IF NOT EXISTS simulated_bets (
  id text PRIMARY KEY,
  candidate_id text REFERENCES candidate_bets(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  hypothetical_stake numeric NOT NULL,
  observed_decimal_odds numeric,
  status text NOT NULL DEFAULT 'open',
  payload jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  event_type text NOT NULL,
  details jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS hermes_reflections (
  id text PRIMARY KEY,
  created_at timestamptz NOT NULL DEFAULT now(),
  title text NOT NULL,
  summary text NOT NULL,
  evidence jsonb NOT NULL,
  status text NOT NULL DEFAULT 'proposed'
);
"""


def utcnow() -> datetime:
    return datetime.now(UTC)


def new_id() -> str:
    return str(uuid.uuid4())


def jsonable(value: Any) -> Any:
    if isinstance(value, datetime):
        return value.isoformat()
    if isinstance(value, dict):
        return {key: jsonable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [jsonable(item) for item in value]
    return value


class Store:
    def __init__(self, database_url: str | None):
        self.database_url = database_url
        self.available = bool(database_url and psycopg)
        self.last_error: str | None = None
        self.memory: dict[str, Any] = {
            "snapshots": [],
            "candidates": [],
            "simulated_bets": [],
            "audit_events": [],
            "hermes_reflections": [],
        }

    def connect(self):
        if not self.available:
            raise RuntimeError("database driver or DATABASE_URL unavailable")
        return psycopg.connect(self.database_url, row_factory=dict_row, connect_timeout=5)

    def init_schema(self) -> None:
        if not self.available:
            return
        try:
            with self.connect() as conn:
                for statement in SCHEMA_SQL.strip().split(";"):
                    if statement.strip():
                        conn.execute(statement)
                conn.commit()
                self.last_error = None
        except Exception as exc:
            self.last_error = str(exc)

    def status(self) -> dict[str, Any]:
        if not self.available:
            return {"available": False, "connected": False, "last_error": self.last_error}
        try:
            with self.connect() as conn:
                row = conn.execute("SELECT 1 AS ok").fetchone()
            self.last_error = None
            return {"available": True, "connected": row["ok"] == 1, "last_error": None}
        except Exception as exc:
            self.last_error = str(exc)
            return {"available": True, "connected": False, "last_error": self.last_error}

    def record_audit(self, event_type: str, details: dict[str, Any]) -> None:
        item = {"id": new_id(), "created_at": utcnow().isoformat(), "event_type": event_type, "details": details}
        if not self.available:
            self.memory["audit_events"].insert(0, item)
            return
        try:
            with self.connect() as conn:
                conn.execute(
                    "INSERT INTO audit_events (id, event_type, details) VALUES (%s, %s, %s)",
                    (item["id"], event_type, Jsonb(details)),
                )
                conn.commit()
            self.last_error = None
        except Exception as exc:
            self.last_error = str(exc)
            self.memory["audit_events"].insert(0, item)

    def save_snapshot(self, payload: dict[str, Any], candidates: list[dict[str, Any]]) -> str:
        snapshot_id = new_id()
        sport_keys = [sport["sport_key"] for sport in payload.get("sports", [])]
        event_count = sum((sport.get("event_count") or 0) + (sport.get("outright_count") or 0) for sport in payload.get("sports", []))
        observed_at = payload.get("observed_at") or utcnow().isoformat()
        for candidate in candidates:
            candidate.setdefault("id", new_id())
            candidate["snapshot_id"] = snapshot_id

        if not self.available:
            self.memory["snapshots"].insert(0, {"id": snapshot_id, "payload": payload, "observed_at": observed_at})
            self.memory["candidates"] = candidates + self.memory["candidates"]
            return snapshot_id

        try:
            with self.connect() as conn:
                conn.execute(
                    """
                    INSERT INTO odds_snapshots (id, observed_at, source, mode, sport_keys, event_count, payload)
                    VALUES (%s, %s, %s, %s, %s, %s, %s)
                    """,
                    (
                        snapshot_id,
                        observed_at,
                        payload.get("source", "unknown"),
                        payload.get("mode", "unknown"),
                        sport_keys,
                        event_count,
                        Jsonb(payload),
                    ),
                )
                for candidate in candidates:
                    conn.execute(
                        """
                        INSERT INTO candidate_bets (
                          id, snapshot_id, sport_key, event_id, event_name, competition,
                          market_id, market_name, market_kind, outcome_id, outcome_name,
                          decimal_odds, rationale, status
                        )
                        VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                        """,
                        (
                            candidate["id"],
                            snapshot_id,
                            candidate["sport_key"],
                            candidate.get("event_id"),
                            candidate.get("event_name"),
                            candidate.get("competition"),
                            candidate.get("market_id"),
                            candidate.get("market_name"),
                            candidate.get("market_kind"),
                            candidate.get("outcome_id"),
                            candidate.get("outcome_name"),
                            candidate.get("decimal_odds"),
                            Jsonb(candidate.get("rationale") or {}),
                            candidate.get("status", "candidate"),
                        ),
                    )
                conn.commit()
            self.last_error = None
            return snapshot_id
        except Exception as exc:
            self.last_error = str(exc)
            self.memory["snapshots"].insert(0, {"id": snapshot_id, "payload": payload, "observed_at": observed_at})
            self.memory["candidates"] = candidates + self.memory["candidates"]
            return snapshot_id

    def latest_snapshot(self) -> dict[str, Any] | None:
        if not self.available:
            return self.memory["snapshots"][0] if self.memory["snapshots"] else None
        try:
            with self.connect() as conn:
                row = conn.execute(
                    "SELECT id, observed_at, payload FROM odds_snapshots ORDER BY observed_at DESC LIMIT 1"
                ).fetchone()
            return dict(row) if row else None
        except Exception as exc:
            self.last_error = str(exc)
            return self.memory["snapshots"][0] if self.memory["snapshots"] else None

    def candidates(self, limit: int = 50) -> list[dict[str, Any]]:
        if not self.available:
            return self.memory["candidates"][:limit]
        try:
            with self.connect() as conn:
                rows = conn.execute(
                    """
                    SELECT id, snapshot_id, created_at, sport_key, event_id, event_name, competition,
                           market_id, market_name, market_kind, outcome_id, outcome_name,
                           decimal_odds::float AS decimal_odds, rationale, status
                    FROM candidate_bets
                    ORDER BY created_at DESC
                    LIMIT %s
                    """,
                    (limit,),
                ).fetchall()
            return [dict(row) for row in rows]
        except Exception as exc:
            self.last_error = str(exc)
            return self.memory["candidates"][:limit]

    def simulate_bet(self, candidate_id: str, stake: float) -> dict[str, Any]:
        candidate = next((item for item in self.candidates(limit=200) if item["id"] == candidate_id), None)
        if not candidate:
            raise ValueError(f"candidate not found: {candidate_id}")
        item = {
            "id": new_id(),
            "candidate_id": candidate_id,
            "created_at": utcnow().isoformat(),
            "hypothetical_stake": stake,
            "observed_decimal_odds": candidate.get("decimal_odds"),
            "status": "open",
            "payload": jsonable({"candidate": candidate, "paper_only": True}),
        }
        if not self.available:
            self.memory["simulated_bets"].insert(0, item)
            return item
        try:
            with self.connect() as conn:
                conn.execute(
                    """
                    INSERT INTO simulated_bets (
                      id, candidate_id, hypothetical_stake, observed_decimal_odds, status, payload
                    )
                    VALUES (%s, %s, %s, %s, %s, %s)
                    """,
                    (
                        item["id"],
                        item["candidate_id"],
                        item["hypothetical_stake"],
                        item["observed_decimal_odds"],
                        item["status"],
                        Jsonb(item["payload"]),
                    ),
                )
                conn.commit()
            self.last_error = None
            return item
        except Exception as exc:
            self.last_error = str(exc)
            self.memory["simulated_bets"].insert(0, item)
            return item

    def simulated_bets(self, limit: int = 50) -> list[dict[str, Any]]:
        if not self.available:
            return self.memory["simulated_bets"][:limit]
        try:
            with self.connect() as conn:
                rows = conn.execute(
                    """
                    SELECT id, candidate_id, created_at, hypothetical_stake::float AS hypothetical_stake,
                           observed_decimal_odds::float AS observed_decimal_odds, status, payload
                    FROM simulated_bets
                    ORDER BY created_at DESC
                    LIMIT %s
                    """,
                    (limit,),
                ).fetchall()
            return [dict(row) for row in rows]
        except Exception as exc:
            self.last_error = str(exc)
            return self.memory["simulated_bets"][:limit]

    def hermes_reflections(self, limit: int = 25) -> list[dict[str, Any]]:
        if not self.available:
            return self.memory["hermes_reflections"][:limit]
        try:
            with self.connect() as conn:
                rows = conn.execute(
                    """
                    SELECT id, created_at, title, summary, evidence, status
                    FROM hermes_reflections
                    ORDER BY created_at DESC
                    LIMIT %s
                    """,
                    (limit,),
                ).fetchall()
            return [dict(row) for row in rows]
        except Exception as exc:
            self.last_error = str(exc)
            return self.memory["hermes_reflections"][:limit]


def dumps(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, default=str)
