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
  implied_probability numeric,
  model_probability numeric,
  expected_value numeric,
  confidence numeric,
  score numeric,
  risk_flags jsonb NOT NULL DEFAULT '[]'::jsonb,
  feature_snapshot jsonb NOT NULL DEFAULT '{}'::jsonb,
  status text NOT NULL DEFAULT 'candidate'
);

ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS implied_probability numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS model_probability numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS expected_value numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS confidence numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS score numeric;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS risk_flags jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE candidate_bets ADD COLUMN IF NOT EXISTS feature_snapshot jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE IF NOT EXISTS simulated_bets (
  id text PRIMARY KEY,
  candidate_id text REFERENCES candidate_bets(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  hypothetical_stake numeric NOT NULL,
  observed_decimal_odds numeric,
  status text NOT NULL DEFAULT 'open',
  strategy_id text NOT NULL DEFAULT 'poc_ranker_v1',
  settled_at timestamptz,
  simulated_return numeric,
  profit_loss numeric,
  settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  payload jsonb NOT NULL
);

ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS strategy_id text NOT NULL DEFAULT 'poc_ranker_v1';
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS settled_at timestamptz;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS simulated_return numeric;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS profit_loss numeric;
ALTER TABLE simulated_bets ADD COLUMN IF NOT EXISTS settlement_payload jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE IF NOT EXISTS settlement_observations (
  id text PRIMARY KEY,
  simulated_bet_id text REFERENCES simulated_bets(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  source text NOT NULL,
  observed_result text NOT NULL,
  confidence numeric NOT NULL,
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
                          decimal_odds, rationale, implied_probability, model_probability,
                          expected_value, confidence, score, risk_flags, feature_snapshot, status
                        )
                        VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
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
                            candidate.get("implied_probability"),
                            candidate.get("model_probability"),
                            candidate.get("expected_value"),
                            candidate.get("confidence"),
                            candidate.get("score"),
                            Jsonb(candidate.get("risk_flags") or []),
                            Jsonb(candidate.get("feature_snapshot") or {}),
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
                           decimal_odds::float AS decimal_odds, rationale,
                           implied_probability::float AS implied_probability,
                           model_probability::float AS model_probability,
                           expected_value::float AS expected_value,
                           confidence::float AS confidence,
                           score::float AS score,
                           risk_flags, feature_snapshot, status
                    FROM candidate_bets
                    ORDER BY created_at DESC, score DESC NULLS LAST
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
            "strategy_id": "poc_ranker_v1",
            "settled_at": None,
            "simulated_return": None,
            "profit_loss": None,
            "settlement_payload": {},
            "payload": jsonable({"candidate": candidate, "paper_only": True, "strategy_id": "poc_ranker_v1"}),
        }
        if not self.available:
            self.memory["simulated_bets"].insert(0, item)
            return item
        try:
            with self.connect() as conn:
                conn.execute(
                    """
                    INSERT INTO simulated_bets (
                      id, candidate_id, hypothetical_stake, observed_decimal_odds, status,
                      strategy_id, settlement_payload, payload
                    )
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
                    """,
                    (
                        item["id"],
                        item["candidate_id"],
                        item["hypothetical_stake"],
                        item["observed_decimal_odds"],
                        item["status"],
                        item["strategy_id"],
                        Jsonb(item["settlement_payload"]),
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
                           observed_decimal_odds::float AS observed_decimal_odds, status,
                           strategy_id, settled_at,
                           simulated_return::float AS simulated_return,
                           profit_loss::float AS profit_loss,
                           settlement_payload, payload
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

    def ledger_summary(self) -> dict[str, Any]:
        bets = self.simulated_bets(limit=1000)
        summary = {
            "count": len(bets),
            "open_count": 0,
            "settled_count": 0,
            "turnover": 0.0,
            "open_exposure": 0.0,
            "simulated_return": 0.0,
            "profit_loss": 0.0,
            "hit_rate": None,
            "average_odds": None,
            "by_status": {},
        }
        won = 0
        decided = 0
        odds_total = 0.0
        odds_count = 0
        for bet in bets:
            status = bet.get("status") or "unknown"
            stake = float(bet.get("hypothetical_stake") or 0)
            returned = float(bet.get("simulated_return") or 0)
            profit_loss = float(bet.get("profit_loss") or 0)
            odds = bet.get("observed_decimal_odds")
            summary["by_status"][status] = summary["by_status"].get(status, 0) + 1
            summary["turnover"] += stake
            summary["simulated_return"] += returned
            summary["profit_loss"] += profit_loss
            if odds is not None:
                odds_total += float(odds)
                odds_count += 1
            if status in {"open", "awaiting_result", "unresolved"}:
                summary["open_count"] += 1
                summary["open_exposure"] += stake
            if status.startswith("settled_") or status in {"void", "pushed"}:
                summary["settled_count"] += 1
            if status in {"settled_won", "settled_lost"}:
                decided += 1
                won += 1 if status == "settled_won" else 0
        if decided:
            summary["hit_rate"] = won / decided
        if odds_count:
            summary["average_odds"] = odds_total / odds_count
        return summary

    def settle_simulated_bet(
        self,
        bet_id: str,
        result: str,
        source: str,
        confidence: float,
        notes: str = "",
    ) -> dict[str, Any]:
        allowed = {
            "won": "settled_won",
            "lost": "settled_lost",
            "void": "void",
            "pushed": "pushed",
            "unresolved": "unresolved",
        }
        if result not in allowed:
            raise ValueError(f"unsupported settlement result: {result}")
        bet = next((item for item in self.simulated_bets(limit=1000) if item["id"] == bet_id), None)
        if not bet:
            raise ValueError(f"simulated bet not found: {bet_id}")
        if bet.get("status") not in {"open", "awaiting_result", "unresolved"}:
            raise ValueError(f"simulated bet is already settled: {bet_id}")

        stake = float(bet.get("hypothetical_stake") or 0)
        odds = float(bet.get("observed_decimal_odds") or 0)
        status = allowed[result]
        if result == "won":
            simulated_return = stake * odds
            profit_loss = simulated_return - stake
        elif result == "lost":
            simulated_return = 0.0
            profit_loss = -stake
        elif result in {"void", "pushed"}:
            simulated_return = stake
            profit_loss = 0.0
        else:
            simulated_return = None
            profit_loss = None

        settlement_payload = {
            "source": source,
            "observed_result": result,
            "confidence": confidence,
            "notes": notes,
            "paper_only": True,
        }
        settled_at = utcnow().isoformat()

        item = {
            **bet,
            "status": status,
            "settled_at": settled_at,
            "simulated_return": simulated_return,
            "profit_loss": profit_loss,
            "settlement_payload": settlement_payload,
        }
        if not self.available:
            self.memory["simulated_bets"] = [item if row["id"] == bet_id else row for row in self.memory["simulated_bets"]]
            return item

        try:
            with self.connect() as conn:
                conn.execute(
                    """
                    UPDATE simulated_bets
                    SET status = %s,
                        settled_at = %s,
                        simulated_return = %s,
                        profit_loss = %s,
                        settlement_payload = %s
                    WHERE id = %s
                    """,
                    (status, settled_at, simulated_return, profit_loss, Jsonb(settlement_payload), bet_id),
                )
                conn.execute(
                    """
                    INSERT INTO settlement_observations (
                      id, simulated_bet_id, source, observed_result, confidence, payload
                    )
                    VALUES (%s, %s, %s, %s, %s, %s)
                    """,
                    (new_id(), bet_id, source, result, confidence, Jsonb(settlement_payload)),
                )
                conn.commit()
            self.last_error = None
            return item
        except Exception as exc:
            self.last_error = str(exc)
            self.memory["simulated_bets"] = [item if row["id"] == bet_id else row for row in self.memory["simulated_bets"]]
            return item

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
