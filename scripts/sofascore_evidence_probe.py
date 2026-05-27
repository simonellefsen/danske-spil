#!/usr/bin/env python3
"""Collect sanitized Sofascore result evidence with agent-browser.

This script intentionally runs outside the scratch container. It reads a public
Sofascore match page through a real browser session and posts only compact result
evidence to the Rust API. It never reads or submits cookies, localStorage, or
credentials.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.error
import urllib.request


EXTRACT_JS = r"""
(() => {
  const text = document.body ? document.body.innerText : "";
  const lines = text.split(/\n+/).map((line) => line.trim()).filter(Boolean);
  const title = document.title || "";
  const titleScore = title.match(/^\s*(.+?)\s+-\s+(.+?)\s+(\d+)\s*:\s*(\d+)/);
  const ftIndex = lines.findIndex((line) => /^FT\b/i.test(line));
  const afterFtNumbers = [];
  if (ftIndex >= 0) {
    for (const line of lines.slice(ftIndex + 1, ftIndex + 32)) {
      if (/^\d+$/.test(line)) {
        afterFtNumbers.push(Number(line));
      } else if (afterFtNumbers.length > 0) {
        break;
      }
      if (afterFtNumbers.length >= 12) break;
    }
  }
  const tennisMatchScore = afterFtNumbers.length >= 8
    ? {
        home_score: afterFtNumbers[afterFtNumbers.length - 2],
        away_score: afterFtNumbers[afterFtNumbers.length - 1],
        raw_numbers_after_ft: afterFtNumbers
      }
    : null;
  return JSON.stringify({
    title,
    url: location.href,
    raw_text_excerpt: lines.slice(0, 120).join("\n").slice(0, 5000),
    title_score: titleScore ? {
      home_name: titleScore[1],
      away_name: titleScore[2],
      home_score: Number(titleScore[3]),
      away_score: Number(titleScore[4])
    } : null,
    tennis_match_score: tennisMatchScore
  });
})()
"""


def run_agent_browser(session_name: str, url: str, wait_ms: int) -> dict:
    base = [
        "agent-browser",
        "--session-name",
        session_name,
        "--allowed-domains",
        "www.sofascore.com,sofascore.com",
    ]
    subprocess.run([*base, "open", url], check=True)
    subprocess.run([*base, "wait", str(wait_ms)], check=True)
    output = subprocess.check_output([*base, "eval", EXTRACT_JS], text=True)
    value = json.loads(output)
    if isinstance(value, str):
        value = json.loads(value)
    if not isinstance(value, dict):
        raise ValueError("agent-browser extraction did not return an object")
    return value


def parse_event_name(event_name: str | None, home_name: str | None, away_name: str | None) -> str:
    if event_name:
        return event_name
    if home_name and away_name:
        return f"{home_name} - {away_name}"
    raise ValueError("--event-name is required unless --home-name and --away-name are provided")


def build_payload(args: argparse.Namespace, extracted: dict) -> dict:
    title_score = extracted.get("title_score") or {}
    tennis_score = extracted.get("tennis_match_score") or {}
    home_name = args.home_name or title_score.get("home_name")
    away_name = args.away_name or title_score.get("away_name")
    event_name = parse_event_name(args.event_name, home_name, away_name)
    home_score = args.home_score
    away_score = args.away_score
    if home_score is None:
        home_score = title_score.get("home_score", tennis_score.get("home_score"))
    if away_score is None:
        away_score = title_score.get("away_score", tennis_score.get("away_score"))
    if home_name is None or away_name is None:
        raise ValueError("--home-name and --away-name are required when the page title has no score")
    if home_score is None or away_score is None:
        raise ValueError("--home-score and --away-score are required when the page score cannot be parsed")

    return {
        "source_key": args.source_key,
        "source_url": extracted.get("url") or args.url,
        "source_title": extracted.get("title"),
        "event_name": event_name,
        "home_name": home_name,
        "away_name": away_name,
        "home_score": int(home_score),
        "away_score": int(away_score),
        "confidence": args.confidence,
        "settle": bool(args.settle),
        "browser_automation": {
            "tool": "agent-browser",
            "session_name": args.session_name,
            "source": "public_match_page",
        },
        "raw_text_excerpt": extracted.get("raw_text_excerpt", ""),
    }


def post_payload(api_base: str, payload: dict) -> dict:
    url = api_base.rstrip("/") + "/api/settlement/external-evidence"
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("url", help="Sofascore match URL")
    parser.add_argument("--api", default="http://127.0.0.1:18083", help="Gambler API base URL")
    parser.add_argument("--session-name", default="danske-spil-sofascore-evidence")
    parser.add_argument("--wait-ms", type=int, default=3500)
    parser.add_argument("--source-key", default="sofascore_results")
    parser.add_argument("--event-name")
    parser.add_argument("--home-name")
    parser.add_argument("--away-name")
    parser.add_argument("--home-score", type=int)
    parser.add_argument("--away-score", type=int)
    parser.add_argument("--confidence", type=float, default=0.82)
    parser.add_argument("--settle", action="store_true", help="Allow deterministic paper settlement")
    parser.add_argument("--dry-run", action="store_true", help="Print payload without POSTing")
    args = parser.parse_args()

    extracted = run_agent_browser(args.session_name, args.url, args.wait_ms)
    payload = build_payload(args, extracted)
    if args.dry_run:
        print(json.dumps({"payload": payload}, indent=2, sort_keys=True))
        return 0
    result = post_payload(args.api, payload)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
