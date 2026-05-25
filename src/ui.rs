use dioxus::prelude::*;

pub fn render_index(base_path: &str) -> String {
    let app = dioxus_ssr::render_element(rsx! {
        div {
            header {
                h1 { "Gambler POC" }
                div { class: "toolbar",
                    button { class: "primary", id: "scan", "Scan markets" }
                    button { id: "refresh", "Refresh" }
                }
            }
            main {
                div { class: "metric-row",
                    div { class: "metric", div { class: "label", "Mode" } div { class: "value", id: "mode", "-" } }
                    div { class: "metric", div { class: "label", "Database" } div { class: "value", id: "database", "-" } }
                    div { class: "metric", div { class: "label", "Latest snapshot" } div { class: "value", id: "snapshot", "-" } }
                    div { class: "metric", div { class: "label", "Catalog events" } div { class: "value", id: "catalog-events", "-" } }
                    div { class: "metric", div { class: "label", "Open exposure" } div { class: "value", id: "exposure", "-" } }
                    div { class: "metric", div { class: "label", "Paper P/L" } div { class: "value", id: "profit", "-" } }
                    div { class: "metric", div { class: "label", "Real-money placement" } div { class: "value danger", id: "placement", "disabled" } }
                }
                div { class: "grid",
                    section {
                        h2 { "Candidate odds" }
                        table {
                            thead { tr {
                                th { "Sport" } th { "Event" } th { "Market" } th { "Outcome" } th { "Odds" } th { "Score" } th {}
                            } }
                            tbody { id: "candidates" }
                        }
                    }
                    section {
                        h2 { "Reasoning" }
                        pre { id: "reasoning", "No scan loaded." }
                    }
                    section {
                        h2 { "Paper ledger" }
                        table {
                            thead { tr {
                                th { "Created" } th { "Selection" } th { "Stake" } th { "Status" } th { "P/L" } th {}
                            } }
                            tbody { id: "ledger" }
                        }
                    }
                    section {
                        h2 { "Market coverage" }
                        table {
                            thead { tr {
                                th { "Sport" } th { "Events" } th { "Competitions" } th { "Markets" } th { "Outcomes" } th { "Candidates" }
                            } }
                            tbody { id: "coverage" }
                        }
                    }
                    section {
                        h2 { "Hermes view" }
                        pre { id: "hermes", "No reflections yet." }
                    }
                }
            }
        }
    });
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Gambler POC</title>
  <style>{style}</style>
</head>
<body data-base-path="{base_path}">
{app}
<script>{script}</script>
</body>
</html>"#,
        style = STYLE,
        script = SCRIPT,
        base_path = html_escape(base_path),
        app = app
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const STYLE: &str = r#"
:root {
  color-scheme: light;
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: #f6f7f9;
  color: #17202a;
}
body { margin: 0; }
header {
  display: flex; align-items: center; justify-content: space-between; gap: 16px;
  padding: 18px 24px; background: #ffffff; border-bottom: 1px solid #d9dee7;
  position: sticky; top: 0; z-index: 1;
}
h1 { margin: 0; font-size: 20px; font-weight: 650; letter-spacing: 0; }
main { max-width: 1280px; margin: 0 auto; padding: 20px 24px 40px; }
.toolbar { display: flex; gap: 10px; flex-wrap: wrap; }
button {
  min-height: 36px; border: 1px solid #aeb8c6; background: #ffffff; color: #17202a;
  border-radius: 6px; padding: 0 12px; font-weight: 600; cursor: pointer;
}
button.primary { background: #1f6feb; color: #ffffff; border-color: #1f6feb; }
button:disabled { opacity: .45; cursor: not-allowed; }
.grid { display: grid; grid-template-columns: 1fr; gap: 16px; }
@media (min-width: 920px) { .grid { grid-template-columns: 1.1fr .9fr; } }
section { background: #ffffff; border: 1px solid #d9dee7; border-radius: 8px; padding: 16px; }
h2 { margin: 0 0 12px; font-size: 16px; letter-spacing: 0; }
.metric-row { display: grid; grid-template-columns: repeat(auto-fit, minmax(155px, 1fr)); gap: 10px; margin-bottom: 16px; }
.metric { border: 1px solid #e2e7ef; border-radius: 6px; padding: 12px; background: #fbfcfe; }
.label { color: #596678; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
.value { margin-top: 5px; font-size: 18px; font-weight: 650; word-break: break-word; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; border-bottom: 1px solid #e6ebf2; padding: 9px 8px; vertical-align: top; }
th { color: #596678; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
.pill { display: inline-block; border: 1px solid #cdd5df; border-radius: 999px; padding: 2px 8px; font-size: 12px; }
.muted { color: #596678; }
.actions { display: flex; gap: 6px; flex-wrap: wrap; }
.actions button { min-height: 28px; padding: 0 8px; font-size: 12px; }
.danger { color: #9f1239; }
.ok { color: #166534; }
pre { white-space: pre-wrap; overflow: auto; max-height: 420px; background: #0f172a; color: #e2e8f0; padding: 12px; border-radius: 6px; }
"#;

const SCRIPT: &str = r#"
const $ = (id) => document.getElementById(id);
const appBase = document.body.dataset.basePath || "";
const api = (path) => `${appBase}${path}`;
const money = (value) => Number(value || 0).toFixed(2);
const pct = (value) => value === null || value === undefined ? "-" : `${(Number(value) * 100).toFixed(1)}%`;
const num = (value) => value === null || value === undefined ? "-" : Number(value).toFixed(3);
const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
}[ch]));
const json = (url, options = {}) => fetch(url, {
  headers: { "content-type": "application/json" },
  ...options
}).then((r) => {
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
  return r.json();
});
function renderRows(items) {
  $("candidates").innerHTML = items.map((item) => `
    <tr>
      <td><span class="pill">${esc(item.sport_key)}</span></td>
      <td>${esc(item.event_name)}<br><span class="label">${esc(item.competition)}</span></td>
      <td>${esc(item.market_name)}<br><span class="label">${esc(item.market_kind)}</span></td>
      <td>${esc(item.outcome_name)}</td>
      <td>${item.decimal_odds ?? ""}<br><span class="muted">imp ${pct(item.implied_probability)}</span></td>
      <td>${num(item.score)}<br><span class="muted">conf ${pct(item.confidence)}</span></td>
      <td><button data-candidate="${item.id}">Paper</button></td>
    </tr>
  `).join("");
  document.querySelectorAll("[data-candidate]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/simulate"), { method: "POST", body: JSON.stringify({ candidate_id: button.dataset.candidate }) });
      await load();
    });
  });
  $("reasoning").textContent = items[0] ? JSON.stringify({
    candidate_id: items[0].id,
    event: items[0].event_name,
    market: items[0].market_name,
    outcome: items[0].outcome_name,
    score: items[0].score,
    implied_probability: items[0].implied_probability,
    model_probability: items[0].model_probability,
    expected_value: items[0].expected_value,
    confidence: items[0].confidence,
    risk_flags: items[0].risk_flags,
    rationale: items[0].rationale,
    feature_snapshot: items[0].feature_snapshot
  }, null, 2) : "No candidates yet.";
}
function renderLedger(items) {
  const candidateLabel = (item) => {
    const candidate = item.payload && item.payload.candidate ? item.payload.candidate : {};
    return `${candidate.event_name || item.candidate_id || ""} / ${candidate.outcome_name || ""}`;
  };
  $("ledger").innerHTML = items.map((item) => `
    <tr>
      <td>${esc(item.created_at)}</td>
      <td>${esc(candidateLabel(item))}<br><span class="label">${esc(item.strategy_id || "")}</span></td>
      <td>${money(item.hypothetical_stake)}<br><span class="muted">@ ${item.observed_decimal_odds ?? "-"}</span></td>
      <td>${esc(item.status)}</td>
      <td>${item.profit_loss === null || item.profit_loss === undefined ? "-" : money(item.profit_loss)}</td>
      <td>
        <div class="actions">
          <button data-settle="${item.id}" data-result="won" ${item.status !== "open" ? "disabled" : ""}>Won</button>
          <button data-settle="${item.id}" data-result="lost" ${item.status !== "open" ? "disabled" : ""}>Lost</button>
          <button data-settle="${item.id}" data-result="void" ${item.status !== "open" ? "disabled" : ""}>Void</button>
        </div>
      </td>
    </tr>
  `).join("");
  document.querySelectorAll("[data-settle]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/ledger/settle"), {
        method: "POST",
        body: JSON.stringify({
          bet_id: button.dataset.settle,
          result: button.dataset.result,
          source: "manual_operator_review",
          confidence: 1
        })
      });
      await load();
    });
  });
}
function renderCoverage(coverage) {
  const sports = coverage.sports || [];
  const totalEvents = sports.reduce((sum, item) => sum + Number(item.event_count || 0), 0);
  $("catalog-events").textContent = String(totalEvents);
  $("coverage").innerHTML = sports.map((item) => `
    <tr>
      <td><span class="pill">${esc(item.sport_key)}</span><br><span class="label">${esc(item.label || "")}</span></td>
      <td>${esc(item.event_count)}</td>
      <td>${esc(item.competition_count)}</td>
      <td>${esc(item.market_count)}</td>
      <td>${esc(item.outcome_count)}</td>
      <td>${esc(item.candidate_count)}</td>
    </tr>
  `).join("");
  if (!sports.length) {
    $("coverage").innerHTML = `<tr><td colspan="6" class="muted">No market catalog rows yet. Run a scan.</td></tr>`;
  }
}
async function load() {
  const status = await json(api("/api/status"));
  $("mode").textContent = status.mode;
  $("database").textContent = status.database.connected ? "connected" : "degraded";
  $("database").className = status.database.connected ? "value ok" : "value danger";
  $("snapshot").textContent = status.latest_snapshot_id || "-";
  $("placement").textContent = status.allow_real_money_placement ? "enabled" : "disabled";
  const summary = await json(api("/api/ledger/summary"));
  $("exposure").textContent = money(summary.open_exposure);
  $("profit").textContent = money(summary.profit_loss);
  $("profit").className = Number(summary.profit_loss || 0) >= 0 ? "value ok" : "value danger";
  const candidates = await json(api("/api/candidates"));
  renderRows(candidates.items || []);
  const ledger = await json(api("/api/ledger"));
  renderLedger(ledger.items || []);
  const coverage = await json(api("/api/catalog/coverage"));
  renderCoverage(coverage);
  const hermes = await json(api("/api/hermes"));
  $("hermes").textContent = JSON.stringify(hermes, null, 2);
}
$("scan").addEventListener("click", async () => {
  $("scan").disabled = true;
  try { await json(api("/api/scan"), { method: "POST", body: "{}" }); await load(); }
  finally { $("scan").disabled = false; }
});
$("refresh").addEventListener("click", load);
load().catch((error) => { $("reasoning").textContent = error.stack || String(error); });
"#;
