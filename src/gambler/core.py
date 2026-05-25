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
                            "decimal_odds": float(odds),
                            "status": "candidate",
                            "rationale": {
                                "paper_only": True,
                                "selection_basis": "First-pass market normalization candidate, not a recommendation.",
                                "safety": "Real-money placement is disabled; candidate can only be paper-ledgered.",
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
                        return candidates
    return candidates


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
