from __future__ import annotations

import os
from urllib.parse import quote
from dataclasses import dataclass


def _bool_env(name: str, default: bool) -> bool:
    value = os.getenv(name)
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


@dataclass(frozen=True)
class Settings:
    component: str
    host: str
    port: int
    database_url: str | None
    observe_only: bool
    allow_real_money_placement: bool
    default_stake: float
    scan_interval_seconds: int
    scan_limit: int
    scan_max_markets: int

    @property
    def mode(self) -> str:
        if self.observe_only and not self.allow_real_money_placement:
            return "observe_only_paper_ledger"
        return "unsafe_real_money_enabled"


def load_settings() -> Settings:
    database_url = os.getenv("DATABASE_URL")
    if not database_url:
        host = os.getenv("DATABASE_HOST")
        name = os.getenv("DATABASE_NAME", "danske_spil")
        user = os.getenv("DATABASE_USER")
        password = os.getenv("DATABASE_PASSWORD")
        port = os.getenv("DATABASE_PORT", "5432")
        if host and user and password:
            database_url = f"postgresql://{quote(user, safe='')}:{quote(password, safe='')}@{host}:{port}/{name}"

    return Settings(
        component=os.getenv("APP_COMPONENT", "gambler-api"),
        host=os.getenv("GAMBLER_HOST", "0.0.0.0"),
        port=int(os.getenv("GAMBLER_PORT", "8080")),
        database_url=database_url,
        observe_only=_bool_env("GAMBLER_OBSERVE_ONLY", True),
        allow_real_money_placement=_bool_env("DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT", False),
        default_stake=float(os.getenv("GAMBLER_DEFAULT_SIMULATED_STAKE", "10")),
        scan_interval_seconds=int(os.getenv("GAMBLER_SCAN_INTERVAL_SECONDS", "900")),
        scan_limit=int(os.getenv("GAMBLER_SCAN_LIMIT", "2")),
        scan_max_markets=int(os.getenv("GAMBLER_SCAN_MAX_MARKETS", "8")),
    )
