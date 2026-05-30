use anyhow::Context;
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};

const CONTENT_BASE: &str = "https://content.sb.danskespil.dk/content-service/api/v1/q";

#[derive(Clone)]
struct SportConfig {
    key: &'static str,
    drilldown_id: &'static str,
    label: &'static str,
    sport_codes: &'static [&'static str],
    outright_drilldown_id: Option<&'static str>,
}

const SPORTS: &[SportConfig] = &[
    SportConfig {
        key: "football",
        drilldown_id: "12",
        label: "Football/soccer",
        sport_codes: &["FOOTBALL"],
        outright_drilldown_id: None,
    },
    SportConfig {
        key: "tennis",
        drilldown_id: "854",
        label: "Tennis",
        sport_codes: &["TENNIS"],
        outright_drilldown_id: None,
    },
    SportConfig {
        key: "basketball",
        drilldown_id: "465",
        label: "Basketball",
        sport_codes: &["BASKETBALL"],
        outright_drilldown_id: None,
    },
    SportConfig {
        key: "motorsports",
        drilldown_id: "319",
        label: "Motorsports",
        sport_codes: &["MOTOR_RACING", "MOTORSPORT"],
        outright_drilldown_id: Some("17711"),
    },
    SportConfig {
        key: "golf",
        drilldown_id: "561",
        label: "Golf",
        sport_codes: &["GOLF"],
        outright_drilldown_id: None,
    },
    SportConfig {
        key: "cycling",
        drilldown_id: "660",
        label: "Cycling",
        sport_codes: &["CYCLING"],
        outright_drilldown_id: None,
    },
];

pub async fn scan_sports(
    limit: usize,
    max_markets: usize,
    include_live: bool,
) -> anyhow::Result<Value> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        .build()?;
    let mut sports = Vec::new();
    for sport in SPORTS {
        sports.push(summarize_sport(&client, sport, limit, max_markets, include_live).await?);
    }
    Ok(json!({
        "source": "content.sb.danskespil.dk content-service",
        "mode": "read_only_anonymous",
        "observed_at": Utc::now(),
        "sports": sports
    }))
}

async fn fetch_json(
    client: &Client,
    path: &str,
    params: &[(&str, String)],
) -> anyhow::Result<Value> {
    let url = format!("{CONTENT_BASE}/{path}");
    let payload = client
        .get(url)
        .query(params)
        .send()
        .await
        .context("content-service request failed")?
        .error_for_status()
        .context("content-service returned non-success")?
        .json::<Value>()
        .await
        .context("content-service JSON decode failed")?;
    Ok(payload)
}

async fn summarize_sport(
    client: &Client,
    config: &SportConfig,
    limit: usize,
    max_markets: usize,
    include_live: bool,
) -> anyhow::Result<Value> {
    let events = fetch_match_events(client, config, limit, max_markets, include_live).await?;
    let outrights = if let Some(drilldown_id) = config.outright_drilldown_id {
        fetch_outrights(client, config, drilldown_id, limit, max_markets).await?
    } else {
        Vec::new()
    };
    let event_count = events.len();
    let outright_count = outrights.len();
    let scoped = events.iter().chain(outrights.iter());
    let mut competitions: Vec<String> = scoped
        .clone()
        .filter_map(|event| {
            event
                .get("competition")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    competitions.sort();
    competitions.dedup();
    let mut market_kinds: Vec<String> = scoped
        .flat_map(|event| {
            event
                .get("markets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|market| {
            market
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    market_kinds.sort();
    market_kinds.dedup();

    Ok(json!({
        "sport_key": config.key,
        "label": config.label,
        "drilldown_id": config.drilldown_id,
        "sport_codes": config.sport_codes,
        "observed_at": Utc::now(),
        "date_days": 0,
        "include_live": include_live,
        "event_count": event_count,
        "outright_count": outright_count,
        "competitions": competitions.into_iter().take(25).collect::<Vec<_>>(),
        "market_kinds": market_kinds,
        "events": events,
        "outrights": outrights
    }))
}

async fn fetch_match_events(
    client: &Client,
    config: &SportConfig,
    limit: usize,
    max_markets: usize,
    include_live: bool,
) -> anyhow::Result<Vec<Value>> {
    let params = vec![
        ("maxMarkets", max_markets.to_string()),
        ("excludeEventsWithNoMarkets", "false".to_string()),
        ("allowedEventSorts", "MTCH".to_string()),
        ("includeChildMarkets", "true".to_string()),
        ("prioritisePrimaryMarkets", "true".to_string()),
        ("includeCommentary", "true".to_string()),
        ("includeIncidents", "true".to_string()),
        ("includeMedia", "true".to_string()),
        ("drilldownTagIds", config.drilldown_id.to_string()),
        (
            "excludeDrilldownTagIds",
            "20769,22796,22797,22800".to_string(),
        ),
        ("useMarketGroupCodeCombis", "true".to_string()),
        ("maxTotalItems", usize::max(limit * 25, 100).to_string()),
        (
            "maxEventsPerCompetition",
            usize::min(usize::max(limit * 4, limit), 50).to_string(),
        ),
        ("maxCompetitionsPerSportPerBand", "20".to_string()),
        ("maxEventsForNextToGo", "5".to_string()),
        ("startTimeOffsetForNextToGo", "600".to_string()),
        ("lang", "da-DK".to_string()),
        ("channel", "I".to_string()),
    ];
    let payload = fetch_json(client, "time-band-event-list", &params).await?;
    let mut events = Vec::new();
    for band in payload
        .pointer("/data/timeBandEvents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for event in band
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !is_relevant_event(event, config, include_live) {
                continue;
            }
            events.push(normalize_event(event, max_markets));
            if events.len() >= limit {
                return Ok(events);
            }
        }
    }
    Ok(events)
}

async fn fetch_outrights(
    client: &Client,
    config: &SportConfig,
    drilldown_id: &str,
    limit: usize,
    max_markets: usize,
) -> anyhow::Result<Vec<Value>> {
    let params = vec![
        ("eventSortsIncluded", "TNMT".to_string()),
        ("includeChildMarkets", "true".to_string()),
        ("drilldownTagIds", drilldown_id.to_string()),
        ("lang", "da-DK".to_string()),
        ("channel", "I".to_string()),
    ];
    let payload = fetch_json(client, "event-list", &params).await?;
    let mut events = Vec::new();
    for event in payload
        .pointer("/data/events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !is_relevant_event(event, config, true) {
            continue;
        }
        events.push(normalize_event(event, max_markets));
        if events.len() >= limit {
            break;
        }
    }
    Ok(events)
}

fn is_relevant_event(event: &Value, config: &SportConfig, include_live: bool) -> bool {
    let sport_code = event
        .pointer("/category/code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !config.sport_codes.is_empty() && !config.sport_codes.contains(&sport_code) {
        return false;
    }
    if !include_live && (bool_field(event, "started") || bool_field(event, "liveNow")) {
        return false;
    }
    let haystack = format!(
        "{} {} {}",
        text_field(event, "name"),
        event
            .pointer("/class/name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        event
            .pointer("/type/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    ![
        "esoccer",
        "ebasketball",
        "efodbold",
        "ebasket",
        "esport",
        "e-sport",
    ]
    .iter()
    .any(|marker| haystack.contains(marker))
}

fn normalize_event(event: &Value, max_markets: usize) -> Value {
    let commentary = event.get("commentary").unwrap_or(&Value::Null);
    let participants = commentary
        .get("participants")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let facts: Vec<Value> = commentary
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|fact| {
            let participant = participants
                .iter()
                .find(|item| item.get("id") == fact.get("participantId"))
                .unwrap_or(&Value::Null);
            json!({
                "type": fact.get("type").cloned().unwrap_or(Value::Null),
                "value": fact.get("value").cloned().unwrap_or(Value::Null),
                "participant": participant.get("name").cloned().unwrap_or(Value::Null),
                "role": participant.get("roleCode").cloned().unwrap_or(Value::Null)
            })
        })
        .collect();

    let markets: Vec<Value> = event
        .get("markets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(max_markets)
        .map(normalize_market)
        .collect();

    json!({
        "id": event.get("id").cloned().unwrap_or(Value::Null),
        "name": event.get("name").cloned().unwrap_or(Value::Null),
        "start_time": event.get("startTime").cloned().unwrap_or(Value::Null),
        "started": event.get("started").cloned().unwrap_or(Value::Null),
        "live_now": event.get("liveNow").cloned().unwrap_or(Value::Null),
        "sort_code": event.get("sortCode").cloned().unwrap_or(Value::Null),
        "status": event.get("status").cloned().unwrap_or(Value::Null),
        "resulted": event.get("resulted").cloned().unwrap_or(Value::Null),
        "settled": event.get("settled").cloned().unwrap_or(Value::Null),
        "sport": event.pointer("/category/name").cloned().unwrap_or(Value::Null),
        "sport_code": event.pointer("/category/code").cloned().unwrap_or(Value::Null),
        "class_name": event.pointer("/class/name").cloned().unwrap_or(Value::Null),
        "competition": event.pointer("/type/name").cloned().unwrap_or(Value::Null),
        "competition_drilldown_tag_id": event.get("competitionDrilldownTagId").cloned().unwrap_or(Value::Null),
        "external_ids": event.get("externalIds").cloned().unwrap_or_else(|| json!([])),
        "teams": event.get("teams").cloned().unwrap_or_else(|| json!([])),
        "market_count": event.get("marketCount").cloned().unwrap_or(Value::Null),
        "scoreboard_facts": facts,
        "markets": markets
    })
}

fn normalize_market(market: &Value) -> Value {
    let outcomes: Vec<Value> = market
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(normalize_outcome)
        .collect();
    json!({
        "id": market.get("id").cloned().unwrap_or(Value::Null),
        "name": market.get("name").cloned().unwrap_or(Value::Null),
        "group_code": market.get("groupCode").cloned().unwrap_or(Value::Null),
        "kind": market_kind(market),
        "status": market.get("status").cloned().unwrap_or(Value::Null),
        "active": market.get("active").cloned().unwrap_or(Value::Null),
        "displayed": market.get("displayed").cloned().unwrap_or(Value::Null),
        "bet_in_run": market.get("betInRun").cloned().unwrap_or(Value::Null),
        "handicap_value": market.get("handicapValue").cloned().unwrap_or(Value::Null),
        "minimum_accumulator": market.get("minimumAccumulator").cloned().unwrap_or(Value::Null),
        "maximum_accumulator": market.get("maximumAccumulator").cloned().unwrap_or(Value::Null),
        "outcome_count": market.get("outcomeCount").cloned().unwrap_or(Value::Null),
        "outcomes": outcomes
    })
}

fn normalize_outcome(outcome: &Value) -> Value {
    let price = outcome
        .get("prices")
        .and_then(Value::as_array)
        .and_then(|prices| prices.first())
        .unwrap_or(&Value::Null);
    let fractional = match (price.get("numerator"), price.get("denominator")) {
        (Some(numerator), Some(denominator)) if !numerator.is_null() && !denominator.is_null() => {
            Value::String(format!("{numerator}/{denominator}"))
        }
        _ => Value::Null,
    };
    json!({
        "id": outcome.get("id").cloned().unwrap_or(Value::Null),
        "name": outcome.get("name").cloned().unwrap_or(Value::Null),
        "type": outcome.get("type").cloned().unwrap_or(Value::Null),
        "sub_type": outcome.get("subType").cloned().unwrap_or(Value::Null),
        "status": outcome.get("status").cloned().unwrap_or(Value::Null),
        "active": outcome.get("active").cloned().unwrap_or(Value::Null),
        "displayed": outcome.get("displayed").cloned().unwrap_or(Value::Null),
        "decimal_odds": price.get("decimal").cloned().unwrap_or(Value::Null),
        "fractional": fractional,
        "handicap_low": price.get("handicapLow").cloned().unwrap_or(Value::Null),
        "handicap_high": price.get("handicapHigh").cloned().unwrap_or(Value::Null)
    })
}

fn market_kind(market: &Value) -> &'static str {
    let name = text_field(market, "name").to_ascii_lowercase();
    let group = text_field(market, "groupCode").to_ascii_lowercase();
    let outcome_text = market
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|outcome| outcome.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let has_handicap = market
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|outcome| {
            outcome
                .get("prices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .any(|price| price.get("handicapLow").is_some() || price.get("handicapHigh").is_some());

    if group.contains("outright") || (name.contains("vinder") && name.contains("championship")) {
        "outright"
    } else if name.contains("special") || group.contains("special") {
        "special"
    } else if name.contains("hjorne")
        || name.contains("hjørne")
        || name.contains("corner")
        || group.contains("corners")
    {
        "corners"
    } else if name.contains("kombination")
        || outcome_text.contains("kombination")
        || group.contains("combi")
    {
        "combination"
    } else if name.contains("over")
        || name.contains("under")
        || group.contains("total")
        || group.contains("over_under")
    {
        "over_under"
    } else if name.contains("mål") || name.contains("mal") || group.contains("goal") {
        "goal"
    } else if name.contains("handicap") || group.contains("handicap") || has_handicap {
        "handicap"
    } else if name.contains("begge hold scorer") || group.contains("both_teams") {
        "both_teams_score"
    } else if name.contains("dobbeltchance") || group.contains("double_chance") {
        "double_chance"
    } else if name.contains("sæt")
        || name.contains("saet")
        || name.contains("set")
        || name.contains("game")
    {
        "set_or_game"
    } else if name.contains("halvleg") || name.contains("half") {
        "half_time"
    } else if name.contains("quarter") || name.contains("periode") {
        "period_or_quarter"
    } else if name.contains("vinder")
        || group.contains("winner")
        || name.contains("kampvinder")
        || group.contains("match_result")
    {
        "winner"
    } else {
        "other"
    }
}

fn text_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or_default()
}

fn bool_field(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}
