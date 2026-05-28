#!/usr/bin/env python3
"""Resolve stale paper selections without operator URL prompts.

The agent reads /api/result-agent/queue, uses configured public result links
when present, and posts sanitized browser evidence through the existing
external-result evidence endpoint. It never places bets and never stores
credentials or browser session material.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parent
EVIDENCE_PROBE = ROOT / "external_result_evidence_probe.py"


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=20) as response:
        return json.loads(response.read().decode("utf-8"))


def source_links(task: dict, browser_only: bool) -> list[dict]:
    links = task.get("source_links") or []
    if browser_only:
        return [link for link in links if link.get("requires_browser_automation")]
    return links


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
