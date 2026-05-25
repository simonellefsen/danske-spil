from __future__ import annotations

import argparse
import time

from .config import load_settings
from .core import GamblerService
from .db import Store


def main() -> int:
    parser = argparse.ArgumentParser(description="Observe-only market scanner worker.")
    parser.add_argument("--once", action="store_true", help="Run one scan and exit.")
    args = parser.parse_args()

    settings = load_settings()
    service = GamblerService(settings, Store(settings.database_url))
    while True:
        result = service.scan(include_live=False)
        print(
            f"scan_completed snapshot_id={result['snapshot_id']} candidates={result['candidate_count']}",
            flush=True,
        )
        if args.once:
            return 0
        time.sleep(settings.scan_interval_seconds)


if __name__ == "__main__":
    raise SystemExit(main())
