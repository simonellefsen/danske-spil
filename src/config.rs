use std::env;

#[derive(Clone, Debug)]
pub struct Settings {
    pub component: String,
    pub host: String,
    pub port: u16,
    pub base_path: String,
    pub mode: String,
    pub observe_only: bool,
    pub allow_real_money_placement: bool,
    pub scan_interval_seconds: u64,
    pub scan_limit: usize,
    pub scan_max_markets: usize,
    pub default_stake: f64,
    pub auto_paper_enabled: bool,
    pub auto_paper_per_scan_limit: usize,
    pub auto_paper_max_open_exposure: f64,
    pub settlement_queue_enabled: bool,
    pub settlement_awaiting_grace_minutes: i64,
    pub settlement_queue_limit: usize,
    pub settlement_lookup_cooldown_minutes: i64,
    pub result_agent_enabled: bool,
    pub result_agent_per_cycle_limit: usize,
    pub result_agent_interval_seconds: u64,
    pub database_url: Option<String>,
}

impl Settings {
    pub fn load() -> Self {
        let database_url = env::var("DATABASE_URL").ok().or_else(|| {
            let host = env::var("DATABASE_HOST").ok()?;
            let port = env::var("DATABASE_PORT").unwrap_or_else(|_| "5432".to_string());
            let name = env::var("DATABASE_NAME").ok()?;
            let user = env::var("DATABASE_USER").ok()?;
            let password = env::var("DATABASE_PASSWORD").ok()?;
            Some(format!("postgres://{user}:{password}@{host}:{port}/{name}"))
        });

        Self {
            component: env::var("APP_COMPONENT").unwrap_or_else(|_| "gambler-api".to_string()),
            host: env::var("GAMBLER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("GAMBLER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),
            base_path: env::var("GAMBLER_BASE_PATH")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            mode: env::var("GAMBLER_MODE")
                .unwrap_or_else(|_| "observe_only_paper_ledger".to_string()),
            observe_only: bool_env("GAMBLER_OBSERVE_ONLY", true),
            allow_real_money_placement: bool_env("DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT", false),
            scan_interval_seconds: env::var("GAMBLER_SCAN_INTERVAL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            scan_limit: env::var("GAMBLER_SCAN_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            scan_max_markets: env::var("GAMBLER_SCAN_MAX_MARKETS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
            default_stake: env::var("GAMBLER_DEFAULT_STAKE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            auto_paper_enabled: bool_env("GAMBLER_AUTO_PAPER_ENABLED", true),
            auto_paper_per_scan_limit: env::var("GAMBLER_AUTO_PAPER_PER_SCAN_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            auto_paper_max_open_exposure: env::var("GAMBLER_AUTO_PAPER_MAX_OPEN_EXPOSURE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            settlement_queue_enabled: bool_env("GAMBLER_SETTLEMENT_QUEUE_ENABLED", true),
            settlement_awaiting_grace_minutes: env::var(
                "GAMBLER_SETTLEMENT_AWAITING_GRACE_MINUTES",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
            settlement_queue_limit: env::var("GAMBLER_SETTLEMENT_QUEUE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            settlement_lookup_cooldown_minutes: env::var(
                "GAMBLER_SETTLEMENT_LOOKUP_COOLDOWN_MINUTES",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15),
            result_agent_enabled: bool_env("GAMBLER_RESULT_AGENT_ENABLED", true),
            result_agent_per_cycle_limit: env::var("GAMBLER_RESULT_AGENT_PER_CYCLE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            result_agent_interval_seconds: env::var("GAMBLER_RESULT_AGENT_INTERVAL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            database_url,
        }
    }
}

fn bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
