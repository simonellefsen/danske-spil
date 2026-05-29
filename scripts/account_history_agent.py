#!/usr/bin/env python3
"""Collect sanitized Danske Spil account-history settlement evidence.

This local-only agent consumes /api/result-agent/account-requests, inspects an
operator-controlled agent-browser session, and posts compact bookmaker status
evidence to /api/settlement/external-evidence. It never prints or stores
credentials, cookies, browser storage, payment data, or full account pages.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


DANSKESPIL_DOMAINS = "danskespil.dk,www.danskespil.dk"
DEFAULT_HISTORY_URL = "https://danskespil.dk/oddset"

EXTRACT_JS = r"""
(() => {
  const text = document.body ? document.body.innerText : "";
  const lines = text.split(/\n+/).map((line) => line.trim()).filter(Boolean);
  return JSON.stringify({
    title: document.title || "",
    url: location.href,
    line_count: lines.length,
    lines
  });
})()
"""

STATUS_PATTERNS = [
    ("lost", ("ikke vundet", "tabt", "lost", "settled lost", "loss")),
    ("refunded", ("refunderet", "refund", "refunded", "money back", "tilbagebetalt")),
    ("cancelled", ("annulleret", "aflyst", "cancelled", "canceled", "cancelled")),
    ("postponed", ("udsat", "postponed", "postpone")),
    ("abandoned", ("afbrudt", "abandoned", "abandon")),
    ("void", ("void", "voided", "annulled")),
    ("pushed", ("push", "pushed", "stake returned", "indsats retur")),
    ("won", ("vundet", "gevinst", "udbetalt", "won", "settled won", "paid out")),
    ("unresolved", ("afventer", "pending", "unresolved", "åben", "open")),
]


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


def history_text_to_extracted(text: str, title: str, url: str | None, session_name: str) -> dict:
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    return {
        "title": title,
        "url": url,
        "line_count": len(lines),
        "lines": lines,
        "session_name": session_name,
    }


def load_extracted_json(path: Path, session_name: str) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("--extracted-json must contain a JSON object")
    if "lines" not in value and isinstance(value.get("text"), str):
        value = history_text_to_extracted(
            value["text"],
            str(value.get("title") or "offline account-history fixture"),
            value.get("url"),
            session_name,
        )
    lines = value.get("lines")
    if not isinstance(lines, list) or not all(isinstance(line, str) for line in lines):
        raise ValueError("--extracted-json must contain string lines or a text field")
    value["line_count"] = len(lines)
    value["session_name"] = session_name
    return value


def sanitize_account_history_url(value: str | None) -> str | None:
    if not value:
        return None
    parsed = urllib.parse.urlparse(value)
    if not parsed.scheme or not parsed.netloc:
        return None
    return urllib.parse.urlunparse((parsed.scheme, parsed.netloc, parsed.path, "", "", ""))


def normalize(value: str | None) -> str:
    if not value:
        return ""
    value = (
        value.replace("Æ", "Ae")
        .replace("æ", "ae")
        .replace("Ø", "Oe")
        .replace("ø", "oe")
        .replace("Å", "Aa")
        .replace("å", "aa")
    )
    decomposed = unicodedata.normalize("NFKD", value)
    asciiish = "".join(char for char in decomposed if not unicodedata.combining(char))
    return re.sub(r"[^a-z0-9]+", " ", asciiish.lower()).strip()


def token_set(value: str | None) -> set[str]:
    stop = {
        "a",
        "ac",
        "bk",
        "fc",
        "if",
        "kk",
        "team",
        "the",
        "vs",
        "v",
        "w",
        "k",
    }
    return {token for token in normalize(value).split() if len(token) > 1 and token not in stop}


def split_event_name(event_name: str | None) -> tuple[str, str] | None:
    if not event_name:
        return None
    for separator in (" - ", " vs ", " v "):
        if separator in event_name:
            home, away = event_name.split(separator, 1)
            return home.strip(), away.strip()
    return None


def side_tokens(event_name: str | None) -> tuple[set[str], set[str]] | None:
    sides = split_event_name(event_name)
    if not sides:
        return None
    home, away = sides
    home_tokens = token_set(home)
    away_tokens = token_set(away)
    if not home_tokens or not away_tokens:
        return None
    return home_tokens, away_tokens


def window_matches_event(window: str, event_names: list[str]) -> bool:
    normalized_window = normalize(window)
    window_tokens = set(normalized_window.split())
    for event_name in event_names:
        normalized_event = normalize(event_name)
        if normalized_event and normalized_event in normalized_window:
            return True
        sides = side_tokens(event_name)
        if sides:
            home_tokens, away_tokens = sides
            if home_tokens <= window_tokens and away_tokens <= window_tokens:
                return True
    return False


def find_context(lines: list[str], event_names: list[str], radius: int) -> str | None:
    for index, _line in enumerate(lines):
        start = max(0, index - radius)
        end = min(len(lines), index + radius + 1)
        window = "\n".join(lines[start:end])
        if window_matches_event(window, event_names):
            return window[:1200]
    return None


def infer_status(context: str) -> tuple[str, str] | None:
    normalized = f" {normalize(context)} "
    matches: list[tuple[str, str]] = []
    for result, phrases in STATUS_PATTERNS:
        for phrase in phrases:
            normalized_phrase = normalize(phrase)
            if normalized_phrase and f" {normalized_phrase} " in normalized:
                matches.append((result, phrase))
                break
    unique_results = {result for result, _phrase in matches}
    if len(unique_results) != 1:
        return None
    result, phrase = matches[0]
    return result, phrase


def request_event_names(request: dict) -> list[str]:
    selection = request.get("selection") or {}
    names = []
    for value in selection.get("event_names") or []:
        if isinstance(value, str) and value.strip():
            names.append(value.strip())
    value = selection.get("event_name")
    if isinstance(value, str) and value.strip():
        names.append(value.strip())
    template_value = (request.get("evidence_template") or {}).get("event_name")
    if isinstance(template_value, str) and template_value.strip():
        names.append(template_value.strip())
    seen = set()
    unique = []
    for name in names:
        key = normalize(name)
        if key and key not in seen:
            seen.add(key)
            unique.append(name)
    return unique


def build_payload(request: dict, result: str, matched_phrase: str, context: str, extracted: dict, settle: bool) -> dict:
    template = dict(request.get("evidence_template") or {})
    selection = request.get("selection") or {}
    ids = request.get("ids") or {}
    event_name = template.get("event_name") or selection.get("event_name")
    payload = {
        "source_key": "danskespil_account_history",
        "bet_id": template.get("bet_id") or ids.get("bet_id"),
        "coupon_simulation_id": template.get("coupon_simulation_id") or ids.get("coupon_simulation_id"),
        "event_name": event_name,
        "sport_key": template.get("sport_key") or selection.get("sport_key"),
        "market_name": template.get("market_name") or selection.get("market_name"),
        "outcome_name": template.get("outcome_name") or selection.get("outcome_name"),
        "settlement_result": result,
        "result_status": result,
        "confidence": 0.95,
        "settle": bool(settle),
        "paper_only": True,
        "browser_automation": {
            "tool": "agent-browser",
            "session_name": extracted.get("session_name"),
            "source": "danskespil_account_history",
            "read_only": True,
        },
        "source_title": extracted.get("title"),
        "source_url": sanitize_account_history_url(extracted.get("url")),
        "raw_text_excerpt": context[:500],
        "matched_status_phrase": matched_phrase,
    }
    return {key: value for key, value in payload.items() if value is not None}


def run_agent_browser(session_name: str, url: str, wait_ms: int, no_open: bool) -> dict:
    command = [
        "agent-browser",
        "--session-name",
        session_name,
        "--allowed-domains",
        DANSKESPIL_DOMAINS,
    ]
    if not no_open:
        subprocess.run([*command, "open", url], check=True)
        subprocess.run([*command, "wait", str(wait_ms)], check=True)
    output = subprocess.check_output([*command, "eval", EXTRACT_JS], text=True)
    value = json.loads(output)
    if isinstance(value, str):
        value = json.loads(value)
    if not isinstance(value, dict):
        raise ValueError("agent-browser extraction did not return an object")
    value["session_name"] = session_name
    return value


def load_extracted(args: argparse.Namespace) -> dict:
    if args.extracted_json:
        return load_extracted_json(Path(args.extracted_json), args.session_name)
    if args.history_text_file:
        text = Path(args.history_text_file).read_text(encoding="utf-8")
        return history_text_to_extracted(
            text,
            "offline account-history text fixture",
            None,
            args.session_name,
        )
    return run_agent_browser(args.session_name, args.history_url, args.wait_ms, args.no_open)


def load_requests(args: argparse.Namespace) -> dict:
    if args.requests_json:
        value = json.loads(Path(args.requests_json).read_text(encoding="utf-8"))
        if isinstance(value, list):
            return {"items": value}
        if not isinstance(value, dict):
            raise ValueError("--requests-json must contain a JSON object or list")
        return value
    api_base = args.api.rstrip("/")
    return fetch_json(f"{api_base}/api/result-agent/account-requests")


def run_once(args: argparse.Namespace) -> dict:
    requests = load_requests(args)
    items = (requests.get("items") or [])[: args.limit]
    extracted = load_extracted(args)
    lines = extracted.get("lines") or []
    results = []
    skipped = []
    for request in items:
        event_names = request_event_names(request)
        if not event_names:
            skipped.append({"reason": "missing_event_name", "request": request.get("ids")})
            continue
        context = find_context(lines, event_names, args.context_radius)
        if not context:
            skipped.append({
                "reason": "event_not_visible_in_account_history",
                "event_names": event_names,
                "request": request.get("ids"),
            })
            continue
        status = infer_status(context)
        if not status:
            skipped.append({
                "reason": "no_deterministic_status_in_context",
                "event_names": event_names,
                "request": request.get("ids"),
            })
            continue
        result, phrase = status
        payload = build_payload(request, result, phrase, context, extracted, args.settle)
        if args.dry_run:
            results.append({"event_names": event_names, "payload": payload, "posted": False})
            continue
        api_base = args.api.rstrip("/")
        response = post_json(f"{api_base}/api/settlement/external-evidence", payload)
        results.append({"event_names": event_names, "payload": payload, "response": response, "posted": True})
    return {
        "paper_only": True,
        "dry_run": bool(args.dry_run),
        "settle": bool(args.settle),
        "request_count": len(items),
        "posted_count": sum(1 for item in results if item.get("posted")),
        "evidence_count": len(results),
        "skipped_count": len(skipped),
        "results": results,
        "skipped": skipped,
        "browser": {
            "title": extracted.get("title"),
            "url": extracted.get("url"),
            "line_count": extracted.get("line_count"),
            "session_name": args.session_name,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", default="http://127.0.0.1:18083", help="Gambler API base URL")
    parser.add_argument(
        "--history-url",
        default=os.getenv("DANSKESPIL_ACCOUNT_HISTORY_URL")
        or os.getenv("DANSKESPIL_LOGIN_URL")
        or DEFAULT_HISTORY_URL,
        help="Danske Spil account/history URL to open in the local browser session",
    )
    parser.add_argument("--session-name", default="danske-spil-account-history")
    parser.add_argument("--wait-ms", type=int, default=5000)
    parser.add_argument("--limit", type=int, default=10)
    parser.add_argument("--context-radius", type=int, default=12)
    parser.add_argument("--no-open", action="store_true", help="Inspect the current session page without navigation")
    parser.add_argument("--requests-json", help="Offline account-request queue JSON fixture")
    parser.add_argument("--extracted-json", help="Offline extracted account-history JSON fixture")
    parser.add_argument("--history-text-file", help="Offline account-history text fixture")
    parser.add_argument("--settle", action="store_true", help="Allow deterministic paper settlement")
    parser.add_argument("--dry-run", action="store_true", help="Print sanitized payloads without POSTing")
    args = parser.parse_args()

    summary = run_once(args)
    print(json.dumps(summary, indent=2, sort_keys=True, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
