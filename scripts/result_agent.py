#!/usr/bin/env python3
"""Resolve stale paper selections without operator URL prompts.

The agent reads /api/result-agent/queue, uses configured public result links
when present, and posts sanitized browser evidence through the existing
external-result evidence endpoint. It never places bets and never stores
credentials or browser session material.
"""

from __future__ import annotations

import argparse
import datetime as dt
import html
import json
import re
import subprocess
import sys
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parent
EVIDENCE_PROBE = ROOT / "external_result_evidence_probe.py"
FLASHSCORE_SEARCH = "https://s.flashscore.com/search/"
FLASHSCORE_BASE = "https://www.flashscore.com"
FLASHSCORE_SOURCE_KEY = "flashscore_results"
FLASHSCORE_SPORT_IDS = {
    "football": 1,
    "soccer": 1,
    "tennis": 2,
    "basketball": 3,
}
FLASHSCORE_SPORT_PATHS = {
    "football": "football",
    "soccer": "football",
    "tennis": "tennis",
    "basketball": "basketball",
}
FINISHED_STAGES = {"3", "10", "11"}
WOMEN_MARKERS = (
    "women",
    "women's",
    "womens",
    "female",
    "dame",
    "damer",
    "damesingle",
    "kvinde",
    "kvinder",
    "wta",
)
MEN_MARKERS = (
    "men",
    "men's",
    "mens",
    "male",
    "herre",
    "herrer",
    "herresingle",
    "atp",
)


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=20) as response:
        return json.loads(response.read().decode("utf-8"))


def post_json(url: str, payload: dict) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"POST {url} failed: HTTP {error.code}: {body}") from error


def source_links(task: dict, browser_only: bool) -> list[dict]:
    links = task.get("source_links") or []
    if browser_only:
        return [link for link in links if link.get("requires_browser_automation")]
    return links


def normalize(value: str | None) -> str:
    if not value:
        return ""
    decomposed = unicodedata.normalize("NFKD", value)
    asciiish = "".join(char for char in decomposed if not unicodedata.combining(char))
    return re.sub(r"[^a-z0-9]+", " ", asciiish.lower()).strip()


def tokens(value: str | None) -> set[str]:
    stop = {"a", "ac", "bk", "fc", "if", "kk", "team", "the"}
    return {token for token in normalize(value).split() if len(token) > 1 and token not in stop}


def infer_gender_scope(selection: dict) -> str | None:
    text = " ".join(
        str(selection.get(key) or "")
        for key in ("competition", "market_name", "event_name", "outcome_name")
    )
    normalized = f" {normalize(text)} "
    if any(re.search(rf"\b{re.escape(normalize(marker))}\b", normalized) for marker in WOMEN_MARKERS):
        return "women"
    if any(re.search(rf"\b{re.escape(normalize(marker))}\b", normalized) for marker in MEN_MARKERS):
        return "men"
    return None


def token_score(query: str, candidate: str) -> float:
    query_tokens = tokens(query)
    candidate_tokens = tokens(candidate)
    if not query_tokens or not candidate_tokens:
        return 0.0
    overlap = len(query_tokens & candidate_tokens)
    return overlap / max(len(query_tokens), 1)


def split_event_name(event_name: str) -> tuple[str, str] | None:
    for sep in (" - ", " vs ", " v "):
        if sep in event_name:
            home, away = event_name.split(sep, 1)
            return home.strip(), away.strip()
    return None


def strip_country_suffix(title: str) -> str:
    return re.sub(r"\s*\([^)]*\)\s*$", "", title).strip()


def parse_jsonp(value: str) -> dict:
    match = re.search(r"\((\{.*\})\)\s*;?\s*$", value, flags=re.DOTALL)
    if not match:
        raise ValueError("Flashscore search response was not JSONP")
    return json.loads(match.group(1))


def flashscore_search_participants(name: str, sport_key: str) -> list[dict]:
    sport_id = FLASHSCORE_SPORT_IDS.get((sport_key or "").lower())
    if not sport_id:
        return []
    params = urllib.parse.urlencode(
        {
            "q": name,
            "l": 1,
            "s": sport_id,
            "f": "1;1;1",
            "pid": 2,
            "sid": 1,
        }
    )
    with urllib.request.urlopen(f"{FLASHSCORE_SEARCH}?{params}", timeout=20) as response:
        data = parse_jsonp(response.read().decode("utf-8", errors="replace"))
    results = [
        item
        for item in data.get("results", [])
        if item.get("type") == "participants" and int(item.get("sport_id", 0)) == sport_id
    ]
    return sorted(results, key=lambda item: token_score(name, item.get("title", "")), reverse=True)


def best_participant(name: str, sport_key: str) -> dict | None:
    for item in flashscore_search_participants(name, sport_key):
        if token_score(name, item.get("title", "")) >= 0.5:
            return item
    return None


def fetch_flashscore_feed_sign(participant: dict, sport_key: str) -> str | None:
    prefix = "player" if (sport_key or "").lower() == "tennis" else "team"
    url = f"{FLASHSCORE_BASE}/{prefix}/{participant['url']}/{participant['id']}/"
    with urllib.request.urlopen(url, timeout=20) as response:
        page = response.read().decode("utf-8", errors="replace")
    match = re.search(r'"feed_sign":"([^"]+)"', page)
    return match.group(1) if match else None


def parse_feed_rows(feed: str) -> list[dict]:
    translations = {
        match.group(1): html.unescape(match.group(2))
        for match in re.finditer(r"LV÷\{([^}]+)\}_([^¬~]*)", feed)
    }
    rows = []
    for raw_row in feed.split("~"):
        row: dict[str, str] = {}
        for cell in raw_row.split("¬"):
            if "÷" not in cell:
                continue
            key, value = cell.split("÷", 1)
            if not key:
                continue
            for placeholder, replacement in translations.items():
                value = value.replace("{" + placeholder + "}", replacement)
            row[key] = html.unescape(value)
        if row:
            rows.append(row)
    return rows


def parse_expected_check_after(task: dict) -> dt.datetime | None:
    value = task.get("expected_result_check_after")
    if not isinstance(value, str):
        return None
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def participant_ids(value: str | None) -> set[str]:
    return {part for part in (value or "").split("/") if part}


def row_side_match(row: dict, home_id: str, away_id: str) -> tuple[bool, bool]:
    home_ids = participant_ids(row.get("PX"))
    away_ids = participant_ids(row.get("PY"))
    normal = home_id in home_ids and away_id in away_ids
    reversed_side = home_id in away_ids and away_id in home_ids
    return normal, reversed_side


def score_event_row(
    row: dict,
    home_name: str,
    away_name: str,
    home_id: str,
    away_id: str,
    expected_check_after: dt.datetime | None,
) -> tuple[float, bool]:
    if "AA" not in row or "AG" not in row or "AH" not in row:
        return 0.0, False
    normal, reversed_side = row_side_match(row, home_id, away_id)
    score = 0.0
    if normal or reversed_side:
        score += 100.0
    feed_home_name = row.get("AE") or row.get("FH") or row.get("WM") or ""
    feed_away_name = row.get("AF") or row.get("FK") or row.get("WN") or ""
    if reversed_side:
        feed_home_name, feed_away_name = feed_away_name, feed_home_name
    score += 25.0 * token_score(home_name, feed_home_name)
    score += 25.0 * token_score(away_name, feed_away_name)
    if expected_check_after and row.get("AD"):
        try:
            start = dt.datetime.fromtimestamp(int(row["AD"]), tz=dt.timezone.utc)
            hours = abs((expected_check_after - start).total_seconds()) / 3600.0
            score += max(0.0, 24.0 - min(hours, 24.0))
        except ValueError:
            pass
    return score, reversed_side


def unresolved(value: str | None) -> bool:
    return not value or "{" in value or "}" in value


def flashscore_match_url_from_participants(
    sport_key: str,
    event_id: str,
    home: dict,
    away: dict,
) -> str:
    sport_path = FLASHSCORE_SPORT_PATHS.get((sport_key or "").lower(), (sport_key or "sport").lower())
    return (
        f"{FLASHSCORE_BASE}/match/{sport_path}/"
        f"{home['url']}-{home['id']}/{away['url']}-{away['id']}/?mid={event_id}"
    )


def flashscore_discover(task: dict) -> dict | None:
    selection = task.get("selection") or {}
    sport_key = str(selection.get("sport_key") or "").lower()
    gender_scope = infer_gender_scope(selection)
    event_name = str(selection.get("event_name") or "")
    sides = split_event_name(event_name)
    if not sides or sport_key not in FLASHSCORE_SPORT_IDS:
        return None
    home_name, away_name = sides
    home = best_participant(home_name, sport_key)
    away = best_participant(away_name, sport_key)
    if not home or not away:
        return None
    feed_sign = fetch_flashscore_feed_sign(home, sport_key)
    if not feed_sign:
        return None
    feed_name = f"pe_2_2_{home['id']}_x"
    request = urllib.request.Request(
        f"{FLASHSCORE_BASE}/x/feed/{feed_name}",
        headers={"x-fsign": feed_sign},
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        rows = parse_feed_rows(response.read().decode("utf-8", errors="replace"))
    expected_check_after = parse_expected_check_after(task)
    scored = [
        (score, reversed_side, row)
        for row in rows
        for score, reversed_side in [score_event_row(row, home_name, away_name, home["id"], away["id"], expected_check_after)]
        if score >= 90.0
    ]
    if not scored:
        return None
    score, reversed_side, row = max(scored, key=lambda item: item[0])
    feed_home = row.get("AE") or row.get("FH") or row.get("WM") or home_name
    feed_away = row.get("AF") or row.get("FK") or row.get("WN") or away_name
    if unresolved(feed_home):
        feed_home = strip_country_suffix(home.get("title", "")) or home_name
    if unresolved(feed_away):
        feed_away = strip_country_suffix(away.get("title", "")) or away_name
    home_score = int(row["AG"])
    away_score = int(row["AH"])
    if reversed_side:
        feed_home, feed_away = feed_away, feed_home
        home_score, away_score = away_score, home_score
    stage = str(row.get("AC") or "")
    source_url = flashscore_match_url_from_participants(sport_key, row["AA"], home, away)
    return {
        "source_key": FLASHSCORE_SOURCE_KEY,
        "source_url": source_url,
        "sport_key": sport_key,
        "gender_scope": gender_scope,
        "event_name": event_name,
        "home_name": feed_home,
        "away_name": feed_away,
        "home_score": home_score,
        "away_score": away_score,
        "event_id": row["AA"],
        "stage": stage,
        "finished": stage in FINISHED_STAGES,
        "confidence": 0.82,
        "match_score": score,
        "home_aliases": [home_name, feed_home, strip_country_suffix(home.get("title", ""))],
        "away_aliases": [away_name, feed_away, strip_country_suffix(away.get("title", ""))],
        "raw_text_excerpt": (
            f"Flashscore feed {feed_name} matched {feed_home} - {feed_away} "
            f"{home_score}:{away_score}; stage={stage}; event_id={row['AA']}"
        ),
    }


def persist_flashscore_discovery(args: argparse.Namespace, evidence: dict) -> dict:
    source_link_payload = {
        "source_key": evidence["source_key"],
        "source_url": evidence["source_url"],
        "sport_key": evidence["sport_key"],
        "gender_scope": evidence.get("gender_scope"),
        "event_name": evidence["event_name"],
        "home_aliases": evidence["home_aliases"],
        "away_aliases": evidence["away_aliases"],
        "requires_browser_automation": False,
        "notes": {
            "agent_discovered": True,
            "agent": "result_agent",
            "method": "flashscore_participant_feed",
            "event_id": evidence["event_id"],
            "stage": evidence["stage"],
        },
    }
    evidence_payload = {
        "source_key": evidence["source_key"],
        "source_url": evidence["source_url"],
        "source_title": (
            f"{evidence['home_name']} - {evidence['away_name']} "
            f"{evidence['home_score']}:{evidence['away_score']}"
        ),
        "event_name": evidence["event_name"],
        "sport_key": evidence["sport_key"],
        "gender_scope": evidence.get("gender_scope"),
        "home_name": evidence["home_name"],
        "away_name": evidence["away_name"],
        "home_aliases": evidence["home_aliases"],
        "away_aliases": evidence["away_aliases"],
        "home_score": evidence["home_score"],
        "away_score": evidence["away_score"],
        "confidence": evidence["confidence"],
        "settle": bool(args.settle),
        "browser_automation": {
            "tool": "direct_http",
            "source": "flashscore_participant_feed",
            "event_id": evidence["event_id"],
        },
        "raw_text_excerpt": evidence["raw_text_excerpt"],
    }
    if args.dry_run:
        return {
            "source_link_payload": source_link_payload,
            "evidence_payload": evidence_payload,
            "posted": False,
        }
    source_link = post_json(args.api.rstrip("/") + "/api/settlement/source-link", source_link_payload)
    evidence_result = None
    if evidence["finished"]:
        evidence_result = post_json(args.api.rstrip("/") + "/api/settlement/external-evidence", evidence_payload)
    return {
        "source_link": source_link,
        "evidence_result": evidence_result,
        "posted": True,
    }


def run_flashscore_discovery(args: argparse.Namespace, task: dict) -> dict:
    evidence = flashscore_discover(task)
    if not evidence:
        raise ValueError("flashscore_discovery_no_match")
    persisted = persist_flashscore_discovery(args, evidence)
    return {
        "source_key": evidence["source_key"],
        "source_url": evidence["source_url"],
        "event_name": evidence["event_name"],
        "home_score": evidence["home_score"],
        "away_score": evidence["away_score"],
        "finished": evidence["finished"],
        "match_score": round(evidence["match_score"], 2),
        **persisted,
    }


def run_public_link_probe(args: argparse.Namespace, task: dict, link: dict) -> dict:
    command = [
        sys.executable,
        str(EVIDENCE_PROBE),
        str(link["source_url"]),
        "--api",
        args.api,
        "--source-key",
        str(link["source_key"]),
        "--session-name",
        args.session_name,
    ]
    event_name = (task.get("selection") or {}).get("event_name")
    if event_name:
        command.extend(["--event-name", str(event_name)])
    if args.settle:
        command.append("--settle")
    if args.dry_run:
        command.append("--dry-run")

    completed = subprocess.run(command, check=True, text=True, capture_output=True)
    return {
        "source_key": link.get("source_key"),
        "source_url": link.get("source_url"),
        "stdout": completed.stdout.strip(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", default="http://127.0.0.1:18083", help="Gambler API base URL")
    parser.add_argument("--limit", type=int, default=10, help="Maximum result-agent tasks to attempt")
    parser.add_argument("--session-name", default="danske-spil-result-agent")
    parser.add_argument("--settle", action="store_true", help="Allow deterministic paper settlement")
    parser.add_argument("--dry-run", action="store_true", help="Print extracted payloads without posting")
    parser.add_argument(
        "--no-discover",
        action="store_true",
        help="Disable automated Flashscore source discovery for tasks without a configured result link",
    )
    parser.add_argument(
        "--browser-only",
        action="store_true",
        help="Only run links that require browser automation; direct HTTP links are handled by the worker",
    )
    args = parser.parse_args()

    queue = fetch_json(args.api.rstrip("/") + "/api/result-agent/queue")
    tasks = (queue.get("items") or [])[: max(args.limit, 0)]
    results = []
    skipped = []
    for task in tasks:
        links = source_links(task, args.browser_only)
        if not links:
            if not args.no_discover:
                try:
                    results.append(run_flashscore_discovery(args, task))
                    continue
                except Exception as error:  # noqa: BLE001 - CLI should report per-task failures
                    skipped.append(
                        {
                            "task_kind": task.get("task_kind"),
                            "reason": "flashscore_discovery_failed",
                            "selection": task.get("selection"),
                            "agent_action": task.get("agent_action"),
                            "error": str(error)[-1000:],
                        }
                    )
                    continue
            skipped.append(
                {
                    "task_kind": task.get("task_kind"),
                    "reason": "no_configured_public_result_link",
                    "selection": task.get("selection"),
                    "agent_action": task.get("agent_action"),
                }
            )
            continue
        for link in links:
            try:
                results.append(run_public_link_probe(args, task, link))
                break
            except subprocess.CalledProcessError as error:
                skipped.append(
                    {
                        "task_kind": task.get("task_kind"),
                        "source_key": link.get("source_key"),
                        "source_url": link.get("source_url"),
                        "reason": "public_probe_failed",
                        "stderr": (error.stderr or "").strip()[-1000:],
                    }
                )

    print(
        json.dumps(
            {
                "attempted_count": len(results),
                "skipped_count": len(skipped),
                "settle": args.settle,
                "dry_run": args.dry_run,
                "results": results,
                "skipped": skipped,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
