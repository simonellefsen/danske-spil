from __future__ import annotations

import json
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from typing import Any


CONTENT_BASE = "https://content.sb.danskespil.dk/content-service/api/v1/q"

SPORTS: dict[str, dict[str, Any]] = {
    "football": {"drilldown_id": "12", "label": "Football/soccer", "sport_codes": {"FOOTBALL"}},
    "tennis": {"drilldown_id": "854", "label": "Tennis", "sport_codes": {"TENNIS"}},
    "basketball": {"drilldown_id": "465", "label": "Basketball", "sport_codes": {"BASKETBALL"}},
    "formula1": {
        "drilldown_id": "319",
        "label": "Formula 1 / motorsport",
        "sport_codes": {"MOTOR_RACING", "MOTORSPORT"},
        "outright_drilldown_id": "17711",
    },
    "golf": {"drilldown_id": "561", "label": "Golf", "sport_codes": {"GOLF"}},
    "cycling": {"drilldown_id": "660", "label": "Cycling", "sport_codes": {"CYCLING"}},
}

VIRTUAL_MARKERS = ("esoccer", "ebasketball", "efodbold", "ebasket", "esport", "e-sport")


def fetch_json(path: str, params: dict[str, Any]) -> dict[str, Any]:
    query = urllib.parse.urlencode(params, doseq=True)
    request = urllib.request.Request(
        f"{CONTENT_BASE}/{path}?{query}",
        headers={
            "Accept": "application/json",
            "User-Agent": (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/125.0.0.0 Safari/537.36"
            ),
        },
    )
    with urllib.request.urlopen(request, timeout=25) as response:
        return json.loads(response.read().decode("utf-8"))


def date_boundaries(days: int) -> str:
    now = datetime.now(timezone.utc).replace(minute=0, second=0, microsecond=0)
    return ",".join((now + timedelta(days=offset)).isoformat().replace("+00:00", "Z") for offset in range(days + 1))


def market_kind(market: dict[str, Any]) -> str:
    name = (market.get("name") or "").lower()
    group = (market.get("groupCode") or "").lower()
    outcome_text = " ".join((outcome.get("name") or "") for outcome in market.get("outcomes") or []).lower()
    price_lines = []
    for outcome in market.get("outcomes") or []:
        for price in outcome.get("prices") or []:
            price_lines.extend([price.get("handicapLow"), price.get("handicapHigh")])

    if "outright" in group or ("vinder" in name and "championship" in name):
        return "outright"
    if "special" in name or "special" in group or "specials" in name:
        return "special"
    if "hjorne" in name or "hjørne" in name or "corner" in name or "corners" in group:
        return "corners"
    if "kombination" in name or "kombination" in outcome_text or "combi" in group:
        return "combination"
    if "over" in name or "under" in name or "total" in group or "over_under" in group or " o " in f" {outcome_text} ":
        return "over_under"
    if "mal" in name or "mål" in name or "goal" in group:
        return "goal"
    if "handicap" in name or "handicap" in group or any(value is not None for value in price_lines):
        return "handicap"
    if "begge hold scorer" in name or "both_teams" in group:
        return "both_teams_score"
    if "dobbeltchance" in name or "double_chance" in group:
        return "double_chance"
    if "saet" in name or "sæt" in name or "set" in name or "game" in name:
        return "set_or_game"
    if "halvleg" in name or "half" in name:
        return "half_time"
    if "quarter" in name or "periode" in name:
        return "period_or_quarter"
    if "vinder" in name or "winner" in group or "kampvinder" in name or "match_result" in group:
        return "winner"
    return "other"


def normalize_outcome(outcome: dict[str, Any]) -> dict[str, Any]:
    prices = outcome.get("prices") or []
    price = prices[0] if prices else {}
    return {
        "id": outcome.get("id"),
        "name": outcome.get("name"),
        "type": outcome.get("type"),
        "sub_type": outcome.get("subType"),
        "status": outcome.get("status"),
        "active": outcome.get("active"),
        "displayed": outcome.get("displayed"),
        "decimal_odds": price.get("decimal"),
        "fractional": (
            f"{price.get('numerator')}/{price.get('denominator')}"
            if price.get("numerator") is not None and price.get("denominator") is not None
            else None
        ),
        "handicap_low": price.get("handicapLow"),
        "handicap_high": price.get("handicapHigh"),
    }


def normalize_market(market: dict[str, Any]) -> dict[str, Any]:
    outcomes = [normalize_outcome(outcome) for outcome in market.get("outcomes") or []]
    return {
        "id": market.get("id"),
        "name": market.get("name"),
        "group_code": market.get("groupCode"),
        "kind": market_kind(market),
        "status": market.get("status"),
        "active": market.get("active"),
        "displayed": market.get("displayed"),
        "bet_in_run": market.get("betInRun"),
        "handicap_value": market.get("handicapValue"),
        "minimum_accumulator": market.get("minimumAccumulator"),
        "maximum_accumulator": market.get("maximumAccumulator"),
        "outcome_count": market.get("outcomeCount"),
        "outcomes": outcomes,
    }


def normalize_event(event: dict[str, Any], max_markets: int) -> dict[str, Any]:
    commentary = event.get("commentary") or {}
    facts = []
    for fact in commentary.get("facts") or []:
        participant = next(
            (item for item in commentary.get("participants") or [] if item.get("id") == fact.get("participantId")),
            {},
        )
        facts.append(
            {
                "type": fact.get("type"),
                "value": fact.get("value"),
                "participant": participant.get("name"),
                "role": participant.get("roleCode"),
            }
        )

    return {
        "id": event.get("id"),
        "name": event.get("name"),
        "start_time": event.get("startTime"),
        "started": event.get("started"),
        "live_now": event.get("liveNow"),
        "sort_code": event.get("sortCode"),
        "status": event.get("status"),
        "resulted": event.get("resulted"),
        "settled": event.get("settled"),
        "sport": (event.get("category") or {}).get("name"),
        "sport_code": (event.get("category") or {}).get("code"),
        "class_name": (event.get("class") or {}).get("name"),
        "competition": (event.get("type") or {}).get("name"),
        "competition_drilldown_tag_id": event.get("competitionDrilldownTagId"),
        "external_ids": event.get("externalIds") or [],
        "teams": event.get("teams") or [],
        "market_count": event.get("marketCount"),
        "scoreboard_facts": facts,
        "markets": [normalize_market(market) for market in (event.get("markets") or [])[:max_markets]],
    }


def is_relevant_event(event: dict[str, Any], config: dict[str, Any], include_live: bool) -> bool:
    sport_code = (event.get("category") or {}).get("code")
    if config.get("sport_codes") and sport_code not in config["sport_codes"]:
        return False
    if not include_live and (event.get("started") or event.get("liveNow")):
        return False
    haystack = " ".join(
        str(value or "")
        for value in (event.get("name"), (event.get("class") or {}).get("name"), (event.get("type") or {}).get("name"))
    ).lower()
    return not any(marker in haystack for marker in VIRTUAL_MARKERS)


def fetch_match_events(
    config: dict[str, Any],
    limit: int,
    max_markets: int,
    date_days: int,
    include_live: bool,
) -> list[dict[str, Any]]:
    params: dict[str, Any] = {
        "maxMarkets": max_markets,
        "excludeEventsWithNoMarkets": "false",
        "allowedEventSorts": "MTCH",
        "includeChildMarkets": "true",
        "prioritisePrimaryMarkets": "true",
        "includeCommentary": "true",
        "includeIncidents": "true",
        "includeMedia": "true",
        "drilldownTagIds": config["drilldown_id"],
        "excludeDrilldownTagIds": "20769,22796,22797,22800",
        "useMarketGroupCodeCombis": "true",
        "maxTotalItems": max(limit * 25, 100),
        "maxEventsPerCompetition": min(max(limit * 4, limit), 50),
        "maxCompetitionsPerSportPerBand": 20,
        "maxEventsForNextToGo": 5,
        "startTimeOffsetForNextToGo": 600,
        "lang": "da-DK",
        "channel": "I",
    }
    if not include_live and date_days > 0:
        params["dates"] = date_boundaries(date_days)

    payload = fetch_json("time-band-event-list", params)
    bands = (((payload.get("data") or {}).get("timeBandEvents")) or [])
    events = []
    for band in bands:
        for event in band.get("events") or []:
            if not is_relevant_event(event, config, include_live):
                continue
            events.append(normalize_event(event, max_markets=max_markets))
            if len(events) >= limit:
                return events
    return events


def fetch_outright_events(config: dict[str, Any], limit: int, max_markets: int) -> list[dict[str, Any]]:
    drilldown_id = config.get("outright_drilldown_id")
    if not drilldown_id:
        return []
    payload = fetch_json(
        "event-list",
        {
            "eventSortsIncluded": "TNMT",
            "includeChildMarkets": "true",
            "drilldownTagIds": drilldown_id,
            "lang": "da-DK",
            "channel": "I",
        },
    )
    events = []
    for event in ((payload.get("data") or {}).get("events")) or []:
        if not is_relevant_event(event, config, include_live=True):
            continue
        events.append(normalize_event(event, max_markets=max_markets))
        if len(events) >= limit:
            break
    return events


def summarize_sport(
    sport_key: str,
    limit: int,
    max_markets: int,
    date_days: int = 0,
    include_live: bool = False,
    include_outrights: bool = True,
) -> dict[str, Any]:
    config = SPORTS[sport_key]
    events = fetch_match_events(config, limit, max_markets, date_days, include_live)
    outrights = fetch_outright_events(config, limit, max_markets) if include_outrights else []
    scoped_events = events + outrights
    competitions = sorted({event.get("competition") for event in scoped_events if event.get("competition")})
    market_kinds = sorted(
        {
            market.get("kind")
            for event in scoped_events
            for market in event.get("markets", [])
            if market.get("kind")
        }
    )
    return {
        "sport_key": sport_key,
        "label": config["label"],
        "drilldown_id": config["drilldown_id"],
        "sport_codes": sorted(config.get("sport_codes") or []),
        "observed_at": datetime.now(timezone.utc).isoformat(),
        "date_days": date_days,
        "include_live": include_live,
        "event_count": len(events),
        "outright_count": len(outrights),
        "competitions": competitions[:25],
        "market_kinds": market_kinds,
        "events": events,
        "outrights": outrights,
    }


def scan_sports(
    sports: list[str] | None = None,
    limit: int = 2,
    max_markets: int = 8,
    include_live: bool = False,
) -> dict[str, Any]:
    selected = sorted(SPORTS.keys()) if not sports else sports
    return {
        "source": "content.sb.danskespil.dk content-service",
        "mode": "read_only_anonymous",
        "observed_at": datetime.now(timezone.utc).isoformat(),
        "sports": [
            summarize_sport(
                sport,
                limit=limit,
                max_markets=max_markets,
                include_live=include_live,
            )
            for sport in selected
        ],
    }
