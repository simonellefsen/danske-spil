from __future__ import annotations

from typing import Any

from .config import Settings
from .danske_spil import scan_sports
from .db import Store


PRIMARY_MARKET_KINDS = {
    "winner",
    "over_under",
    "handicap",
    "both_teams_score",
    "double_chance",
    "set_or_game",
    "period_or_quarter",
    "half_time",
    "corners",
    "goal",
    "outright",
}


MARKET_KIND_BASELINES = {
    "winner": 0.03,
    "over_under": 0.015,
    "handicap": 0.01,
    "both_teams_score": 0.0,
    "double_chance": -0.005,
    "set_or_game": -0.02,
    "period_or_quarter": -0.025,
    "half_time": -0.02,
    "corners": -0.03,
    "goal": -0.015,
    "outright": -0.04,
}


def candidate_features(sport: dict[str, Any], event: dict[str, Any], market: dict[str, Any], outcome: dict[str, Any]) -> dict[str, Any]:
    teams = event.get("teams") or []
    start_time = event.get("start_time")
    scoreboard_facts = event.get("scoreboard_facts") or []
    risk_flags = []
    if not start_time:
        risk_flags.append("missing_start_time")
    if not teams and market.get("kind") != "outright":
        risk_flags.append("missing_participants")
    if not scoreboard_facts and event.get("live_now"):
        risk_flags.append("missing_live_scoreboard")
    if market.get("kind") in {"corners", "goal", "period_or_quarter", "set_or_game", "half_time"}:
        risk_flags.append("specialized_market")
    if market.get("kind") == "outright":
        risk_flags.append("long_horizon_market")
    if outcome.get("handicap_low") is not None or outcome.get("handicap_high") is not None:
        risk_flags.append("line_market")

    return {
        "source": "danskespil_content_service",
        "sport_key": sport.get("sport_key"),
        "sport_label": sport.get("label"),
        "competition": event.get("competition"),
        "class_name": event.get("class_name"),
        "start_time": start_time,
        "live_now": bool(event.get("live_now")),
        "started": bool(event.get("started")),
        "team_count": len(teams),
        "scoreboard_fact_count": len(scoreboard_facts),
        "market_kind": market.get("kind"),
        "market_group_code": market.get("group_code"),
        "handicap_low": outcome.get("handicap_low"),
        "handicap_high": outcome.get("handicap_high"),
        "risk_flags": risk_flags,
    }


def score_candidate(decimal_odds: float, features: dict[str, Any]) -> dict[str, Any]:
    implied_probability = 1 / decimal_odds
    risk_flags = list(features.get("risk_flags") or [])
    if decimal_odds < 1.25:
        risk_flags.append("very_short_price")
    if decimal_odds > 8:
        risk_flags.append("long_price")

    completeness = 0.35
    completeness += 0.18 if features.get("start_time") else 0
    completeness += 0.18 if features.get("competition") else 0
    completeness += 0.14 if features.get("team_count") else 0
    completeness += 0.10 if features.get("market_group_code") else 0
    completeness += 0.05 if features.get("scoreboard_fact_count") else 0
    confidence = max(0.1, min(0.82, completeness - (0.04 * len(risk_flags))))

    kind_adjustment = MARKET_KIND_BASELINES.get(str(features.get("market_kind")), -0.03)
    odds_penalty = 0.04 if decimal_odds > 5 else 0.0
    model_probability = max(
        0.01,
        min(0.95, implied_probability + kind_adjustment - odds_penalty - (0.01 * len(risk_flags))),
    )
    expected_value = (model_probability * decimal_odds) - 1
    score = (expected_value * confidence) - (0.015 * len(risk_flags))
    return {
        "implied_probability": implied_probability,
        "model_probability": model_probability,
        "expected_value": expected_value,
        "confidence": confidence,
        "score": score,
        "risk_flags": sorted(set(risk_flags)),
    }


def build_candidates(snapshot: dict[str, Any], max_candidates: int = 40) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    for sport in snapshot.get("sports", []):
        for event in (sport.get("events") or []) + (sport.get("outrights") or []):
            for market in event.get("markets") or []:
                if market.get("kind") not in PRIMARY_MARKET_KINDS:
                    continue
                if not market.get("displayed"):
                    continue
                for outcome in market.get("outcomes") or []:
                    odds = outcome.get("decimal_odds")
                    if odds is None or not outcome.get("displayed"):
                        continue
                    decimal_odds = float(odds)
                    features = candidate_features(sport, event, market, outcome)
                    scoring = score_candidate(decimal_odds, features)
                    candidates.append(
                        {
                            "sport_key": sport["sport_key"],
                            "event_id": event.get("id"),
                            "event_name": event.get("name"),
                            "competition": event.get("competition"),
                            "market_id": market.get("id"),
                            "market_name": market.get("name"),
                            "market_kind": market.get("kind"),
                            "outcome_id": outcome.get("id"),
                            "outcome_name": outcome.get("name"),
                            "decimal_odds": decimal_odds,
                            "implied_probability": scoring["implied_probability"],
                            "model_probability": scoring["model_probability"],
                            "expected_value": scoring["expected_value"],
                            "confidence": scoring["confidence"],
                            "score": scoring["score"],
                            "risk_flags": scoring["risk_flags"],
                            "feature_snapshot": features,
                            "status": "candidate",
                            "rationale": {
                                "paper_only": True,
                                "strategy_id": "poc_ranker_v1",
                                "selection_basis": "Conservative watchlist score from odds shape and available market metadata; not a recommendation.",
                                "safety": "Real-money placement is disabled; candidate can only be paper-ledgered.",
                                "score_summary": {
                                    "implied_probability": scoring["implied_probability"],
                                    "model_probability": scoring["model_probability"],
                                    "expected_value": scoring["expected_value"],
                                    "confidence": scoring["confidence"],
                                    "score": scoring["score"],
                                    "risk_flags": scoring["risk_flags"],
                                },
                                "evidence": {
                                    "sport": sport.get("label"),
                                    "competition": event.get("competition"),
                                    "market_kind": market.get("kind"),
                                    "market_group_code": market.get("group_code"),
                                    "start_time": event.get("start_time"),
                                    "scoreboard_facts": event.get("scoreboard_facts") or [],
                                    "handicap_low": outcome.get("handicap_low"),
                                    "handicap_high": outcome.get("handicap_high"),
                                },
                            },
                        }
                    )
                    if len(candidates) >= max_candidates:
                        return sorted(candidates, key=lambda item: item.get("score") or 0, reverse=True)
    return sorted(candidates, key=lambda item: item.get("score") or 0, reverse=True)


class GamblerService:
    def __init__(self, settings: Settings, store: Store):
        self.settings = settings
        self.store = store
        self.store.init_schema()

    def status(self) -> dict[str, Any]:
        latest = self.store.latest_snapshot()
        candidates = self.store.candidates(limit=5)
        ledger = self.store.simulated_bets(limit=5)
        return {
            "component": self.settings.component,
            "mode": self.settings.mode,
            "observe_only": self.settings.observe_only,
            "allow_real_money_placement": self.settings.allow_real_money_placement,
            "database": self.store.status(),
            "latest_snapshot_id": latest.get("id") if latest else None,
            "recent_candidate_count": len(candidates),
            "recent_simulated_bet_count": len(ledger),
            "ledger_summary": self.store.ledger_summary(),
            "strategy_id": "poc_ranker_v1",
            "sports_scope": ["football", "tennis", "basketball", "formula1", "golf", "cycling"],
        }

    def scan(self, include_live: bool = False) -> dict[str, Any]:
        snapshot = scan_sports(
            limit=self.settings.scan_limit,
            max_markets=self.settings.scan_max_markets,
            include_live=include_live,
        )
        candidates = build_candidates(snapshot)
        snapshot_id = self.store.save_snapshot(snapshot, candidates)
        self.store.record_audit(
            "scan_completed",
            {
                "snapshot_id": snapshot_id,
                "candidate_count": len(candidates),
                "include_live": include_live,
                "paper_only": True,
            },
        )
        return {"snapshot_id": snapshot_id, "candidate_count": len(candidates), "snapshot": snapshot}
