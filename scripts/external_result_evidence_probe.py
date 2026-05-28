#!/usr/bin/env python3
"""Collect sanitized browser result evidence from public match pages.

The script runs locally with agent-browser and posts only compact evidence to
the Rust API. It does not read cookies, localStorage, credentials, or account
payloads. Settlement remains opt-in through --settle.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request


SOURCES = {
    "flashscore_results": {
        "domains": "www.flashscore.com,flashscore.com",
        "confidence": 0.82,
    },
    "sofascore_results": {
        "domains": "www.sofascore.com,sofascore.com",
        "confidence": 0.82,
    },
    "livescore_results": {
        "domains": "www.livescore.com,livescore.com",
        "confidence": 0.80,
    },
}

EXTRACT_JS = r"""
(() => {
  const text = document.body ? document.body.innerText : "";
  const lines = text.split(/\n+/).map((line) => line.trim()).filter(Boolean);
  const meta = {};
  for (const node of document.querySelectorAll("meta")) {
    const key = node.getAttribute("property") || node.getAttribute("name");
    const value = node.getAttribute("content");
    if (key && value) meta[key] = value;
  }
  const scoreTexts = Array.from(document.querySelectorAll(
    "[class*='score'], [class*='Score'], [class*='detailScore'], [class*='duelParticipant']"
  )).map((node) => node.innerText || node.textContent || "")
    .map((value) => value.trim())
    .filter(Boolean)
    .slice(0, 80);

  const ftIndex = lines.findIndex((line) => /^FT\b/i.test(line));
  const afterFtNumbers = [];
  if (ftIndex >= 0) {
    for (const line of lines.slice(ftIndex + 1, ftIndex + 40)) {
      if (/^\d+$/.test(line)) {
        afterFtNumbers.push(Number(line));
      } else if (afterFtNumbers.length > 0) {
        break;
      }
      if (afterFtNumbers.length >= 16) break;
    }
  }

  return JSON.stringify({
    title: document.title || "",
    url: location.href,
    meta,
    score_texts: scoreTexts,
    raw_text_excerpt: lines.slice(0, 140).join("\n").slice(0, 5000),
    numbers_after_ft: afterFtNumbers
  });
})()
"""


def infer_source_key(url: str) -> str:
    host = urllib.parse.urlparse(url).hostname or ""
    if "flashscore." in host:
        return "flashscore_results"
    if "sofascore." in host:
        return "sofascore_results"
    if "livescore." in host:
        return "livescore_results"
    return "documented_third_party_results"


def run_agent_browser(session_name: str, url: str, wait_ms: int, source_key: str) -> dict:
    domains = SOURCES.get(source_key, {}).get("domains")
    command = ["agent-browser", "--session-name", session_name]
    if domains:
        command.extend(["--allowed-domains", domains])
    subprocess.run([*command, "open", url], check=True)
    subprocess.run([*command, "wait", str(wait_ms)], check=True)
    output = subprocess.check_output([*command, "eval", EXTRACT_JS], text=True)
    value = json.loads(output)
    if isinstance(value, str):
        value = json.loads(value)
    if not isinstance(value, dict):
        raise ValueError("agent-browser extraction did not return an object")
    return value


def parse_score_label(value: str | None) -> tuple[str, str, int, int] | None:
    if not value:
        return None
    clean = re.sub(r"\s+", " ", value.replace("\xa0", " ")).strip()
    patterns = [
        r"^(.+?)\s+-\s+(.+?)\s+(\d+)\s*:\s*(\d+)(?:\D.*)?$",
        r"^(.+?)\s+v\s+(.+?)\s+(\d+)\s*:\s*(\d+)(?:\D.*)?$",
        r"^(.+?)\s+vs\.?\s+(.+?)\s+(\d+)\s*:\s*(\d+)(?:\D.*)?$",
    ]
    for pattern in patterns:
        match = re.match(pattern, clean, flags=re.IGNORECASE)
        if match:
            return (
                match.group(1).strip(),
                match.group(2).strip(),
                int(match.group(3)),
                int(match.group(4)),
            )
    return None


def parse_event_label(value: str | None) -> tuple[str, str] | None:
    if not value:
        return None
    clean = re.sub(r"\s+", " ", value.replace("\xa0", " ")).strip()
    patterns = [
        r"^(.+?)\s+v\s+(.+?)\s+\(?\d{1,2}/\d{1,2}/\d{4}\)?(?:\s+\|.*)?$",
        r"^(.+?)\s+-\s+(.+?)\s+\(?\d{1,2}/\d{1,2}/\d{4}\)?(?:\s+\|.*)?$",
        r"^(.+?)\s+vs\.?\s+(.+?)(?:\s+\|.*)?$",
    ]
    for pattern in patterns:
        match = re.match(pattern, clean, flags=re.IGNORECASE)
        if match:
            return match.group(1).strip(), match.group(2).strip()
    return None


def extract_names_and_score(args: argparse.Namespace, extracted: dict) -> tuple[str, str, int, int]:
    labels = [
        extracted.get("title"),
        (extracted.get("meta") or {}).get("og:title"),
        (extracted.get("meta") or {}).get("twitter:title"),
        *(extracted.get("score_texts") or []),
    ]
    parsed_score = next((parsed for label in labels if (parsed := parse_score_label(label))), None)
    parsed_event = next((parsed for label in labels if (parsed := parse_event_label(label))), None)

    home_name = args.home_name or (parsed_score[0] if parsed_score else None) or (
        parsed_event[0] if parsed_event else None
    )
    away_name = args.away_name or (parsed_score[1] if parsed_score else None) or (
        parsed_event[1] if parsed_event else None
    )
    home_score = args.home_score if args.home_score is not None else (
        parsed_score[2] if parsed_score else None
    )
    away_score = args.away_score if args.away_score is not None else (
        parsed_score[3] if parsed_score else None
    )

    numbers_after_ft = extracted.get("numbers_after_ft") or []
    if (
        (home_score is None or away_score is None)
        and len(numbers_after_ft) >= 8
        and args.source_key == "sofascore_results"
    ):
        home_score = numbers_after_ft[-2]
        away_score = numbers_after_ft[-1]

    missing = []
    if not home_name:
        missing.append("--home-name")
    if not away_name:
        missing.append("--away-name")
    if home_score is None:
        missing.append("--home-score")
    if away_score is None:
        missing.append("--away-score")
    if missing:
        raise ValueError(
            "could not parse result evidence; provide " + ", ".join(missing)
        )

    return str(home_name), str(away_name), int(home_score), int(away_score)


def build_payload(args: argparse.Namespace, extracted: dict) -> dict:
    home_name, away_name, home_score, away_score = extract_names_and_score(args, extracted)
    event_name = args.event_name or f"{home_name} - {away_name}"
    return {
        "source_key": args.source_key,
        "source_url": extracted.get("url") or args.url,
        "source_title": extracted.get("title"),
        "event_name": event_name,
        "home_name": home_name,
        "away_name": away_name,
        "home_score": home_score,
        "away_score": away_score,
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
    parser.add_argument("url", help="Public match result URL")
    parser.add_argument("--api", default="http://127.0.0.1:18083", help="Gambler API base URL")
    parser.add_argument("--source-key", choices=sorted(SOURCES), help="Settlement source key")
    parser.add_argument("--session-name", default="danske-spil-result-evidence")
    parser.add_argument("--wait-ms", type=int, default=3500)
    parser.add_argument("--event-name")
    parser.add_argument("--home-name")
    parser.add_argument("--away-name")
    parser.add_argument("--home-score", type=int)
    parser.add_argument("--away-score", type=int)
    parser.add_argument("--confidence", type=float)
    parser.add_argument("--settle", action="store_true", help="Allow deterministic paper settlement")
    parser.add_argument("--dry-run", action="store_true", help="Print payload without POSTing")
    args = parser.parse_args()

    args.source_key = args.source_key or infer_source_key(args.url)
    if args.source_key not in SOURCES:
        raise ValueError(f"unsupported browser evidence source: {args.source_key}")
    if args.confidence is None:
        args.confidence = SOURCES[args.source_key]["confidence"]

    extracted = run_agent_browser(args.session_name, args.url, args.wait_ms, args.source_key)
    payload = build_payload(args, extracted)
    if args.dry_run:
        print(json.dumps({"payload": payload}, indent=2, sort_keys=True))
        return 0
    result = post_payload(args.api, payload)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
