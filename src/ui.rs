use dioxus::prelude::*;

pub fn render_index(base_path: &str) -> String {
    let app = dioxus_ssr::render_element(rsx! {
        div {
            header {
                h1 { "Gambler POC" }
                div { class: "toolbar",
                    button { class: "primary", id: "scan", title: "Fetch the latest observed markets, update candidates, apply strategy gates, and refresh paper-settlement state.", "Scan markets" }
                    button { id: "auto-paper", title: "Create paper-ledger entries for currently selected single-bet candidates. This never places real bets.", "Auto paper selected" }
                    button { id: "generate-coupons", title: "Build provider-compatible paper coupon candidates such as doubles or triples from selected legs.", "Generate coupons" }
                    button { id: "auto-paper-coupons", title: "Create paper-ledger coupon simulations for selected coupon candidates. This never places real bets.", "Auto paper coupons" }
                    button { id: "queue-settlement", title: "Move open paper bets/coupons whose expected result-check time has passed into awaiting-result review.", "Queue settlement" }
                    button { id: "review-settlement", title: "Refresh result evidence, stale lookup state, source recommendations, and result-agent tasks for awaiting paper positions.", "Review results" }
                    button { id: "run-result-agent", title: "Run the read-only result agent now. It discovers public result links and posts sanitized paper-settlement evidence without placing bets.", "Run result agent" }
                    button { id: "commit-settlements", title: "Apply the settlement outcomes you selected in Settlement review. Disabled until at least one row is selected.", disabled: true, "Commit selected settlements" }
                    button { id: "reflect-yesterday", title: "Record or refresh the Hermes-safe previous-day paper-performance reflection.", "Reflect yesterday" }
                    button { id: "run-hermes", title: "Run one Hermes-safe loop cycle now: refresh the daily paper reflection, refresh replay evidence for open one-variable experiments, and summarize promotion gates. This cannot control the browser or place bets.", "Run Hermes" }
                    button { id: "refresh", title: "Reload dashboard data without triggering a market scan.", "Refresh" }
                }
            }
            main {
                div { class: "metric-row",
                    div { class: "metric", div { class: "label", "Mode" } div { class: "value", id: "mode", "-" } }
                    div { class: "metric", div { class: "label", "Database" } div { class: "value", id: "database", "-" } }
                    div { class: "metric", div { class: "label", "Latest snapshot" } div { class: "value", id: "snapshot", "-" } }
                    div { class: "metric", div { class: "label", "Scan cadence" } div { class: "value", id: "scan-cadence", "-" } }
                    div { class: "metric", div { class: "label", "Next scan due" } div { class: "value", id: "next-scan-due", "-" } }
                    div { class: "metric", div { class: "label", "Catalog events" } div { class: "value", id: "catalog-events", "-" } }
                    div { class: "metric", div { class: "label", "Coupon rules" } div { class: "value", id: "coupon-rules-count", "-" } }
                    div { class: "metric", div { class: "label", "Odds moves" } div { class: "value", id: "odds-moves-count", "-" } }
                    div { class: "metric", div { class: "label", "Feature snapshots" } div { class: "value", id: "feature-snapshots", "-" } }
                    div { class: "metric", div { class: "label", "Strategy selected" } div { class: "value", id: "strategy-selected", "-" } }
                    div { class: "metric", div { class: "label", "Strategy rejected" } div { class: "value", id: "strategy-rejected", "-" } }
                    div { class: "metric", div { class: "label", "Auto paper" } div { class: "value", id: "auto-paper-state", "-" } }
                    div { class: "metric", div { class: "label", "Next capacity" } div { class: "value", id: "next-capacity", "-" } }
                    div { class: "metric", div { class: "label", "Awaiting result" } div { class: "value", id: "awaiting-result", "-" } }
                    div { class: "metric", div { class: "label", "Due review" } div { class: "value", id: "due-review", "-" } }
                    div { class: "metric", div { class: "label", "Lookup due" } div { class: "value", id: "lookup-due", "-" } }
                    div { class: "metric", div { class: "label", "Result agent" } div { class: "value", id: "result-agent-tasks", "-" } }
                    div { class: "metric", div { class: "label", "Hermes loop" } div { class: "value", id: "hermes-loop", "-" } }
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
                        h2 { "Strategy decisions" }
                        table {
                            thead { tr {
                                th { "Decision" } th { "Selection" } th { "Odds" } th { "Score" } th { "Reasons" }
                            } }
                            tbody { id: "strategy-decisions" }
                        }
                    }
                    section {
                        h2 { "Candidate coupons" }
                        table {
                            thead { tr {
                                th { "Type" } th { "Legs" } th { "Combined odds" } th { "Score" } th { "Rule evidence" } th {}
                            } }
                            tbody { id: "coupons" }
                        }
                    }
                    section {
                        h2 { "Simulated coupons" }
                        table {
                            thead { tr {
                                th { "Created" } th { "Coupon" } th { "Stake" } th { "Expected" } th { "Status" } th { "P/L" } th {}
                            } }
                            tbody { id: "simulated-coupons" }
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
                                th { "Created" } th { "Selection" } th { "Stake" } th { "Expected" } th { "Status" } th { "P/L" } th {}
                            } }
                            tbody { id: "ledger" }
                        }
                    }
                    section {
                        h2 { title: "Manual review surface for paper positions. Select an outcome here only after evidence is known, then commit selected settlements.", "Settlement review" }
                        table {
                            thead { tr {
                                th { "Selection" } th { "Expected finish" } th { "Event state" } th { "Recommendation" }
                            } }
                            tbody { id: "settlement-review" }
                        }
                    }
                    section {
                        h2 { title: "Automation backlog for stale paper positions. Rows are not clicked directly: the worker handles direct configured links, and local result-agent/account-history agents should consume this queue and post sanitized evidence.", "Result agent queue" }
                        table {
                            thead { tr {
                                th { "Task" } th { "Selection" } th { "Sources" } th { "Agent action" }
                            } }
                            tbody { id: "result-agent-queue" }
                        }
                    }
                    section {
                        h2 { title: "Latest scheduled or manual read-only result-agent cycle, including queue priority and paper-exposure accounting.", "Result-agent cycle" }
                        table {
                            thead { tr {
                                th { "Completed" } th { "Queued" } th { "Selected" } th { "Attempted" } th { "Outcome" }
                            } }
                            tbody { id: "result-agent-cycle" }
                        }
                    }
                    section {
                        h2 { title: "Recent scheduled and manual result-agent runs. Use this to confirm the 15-minute reconciliation loop is progressing and whether cycles are settling or skipping backlog rows.", "Recent result-agent cycles" }
                        table {
                            thead { tr {
                                th { "Completed" } th { "Queued" } th { "Selected" } th { "Attempted" } th { "Skipped" } th { "Settled" }
                            } }
                            tbody { id: "result-agent-cycle-history" }
                        }
                    }
                    section {
                        h2 { title: "Focused worklist for a local read-only Danske Spil account-history browser agent. It lists settlement facts to inspect and forbids credentials, cookies, browser storage, and full account pages.", "Account-history requests" }
                        div { class: "operator-note", id: "account-history-agent-runbook", "Loading local account-history agent runbook..." }
                        table {
                            thead { tr {
                                th { "Request" } th { "Selection" } th { "Expected truth" } th { "Evidence contract" }
                            } }
                            tbody { id: "account-history-requests" }
                        }
                    }
                    section {
                        h2 { title: "Ordered source policy used for settlement evidence. Account history is preferred, then official results, then public result pages.", "Settlement sources" }
                        table {
                            thead { tr {
                                th { "Priority" } th { "Source" } th { "Scope" } th { "Reliability" } th { "Notes" }
                            } }
                            tbody { id: "settlement-sources" }
                        }
                    }
                    section {
                        h2 { title: "Persisted event-to-result-page links. Direct links can be auto-checked by the worker; browser-only links are consumed by result-agent probes.", "Operator result links" }
                        table {
                            thead { tr {
                                th { "Updated" } th { "Event" } th { "Source" } th { "Aliases" } th { "Mode" }
                            } }
                            tbody { id: "external-result-links" }
                        }
                    }
                    section {
                        h2 { title: "Reusable names learned across sources for teams, players, leagues, and other participants. Settlement matching expands aliases from this registry.", "Alias registry" }
                        table {
                            thead { tr {
                                th { "Updated" } th { "Entity" } th { "Alias" } th { "Source" } th { "Confidence" }
                            } }
                            tbody { id: "entity-aliases" }
                        }
                    }
                    section {
                        h2 { "Settlement observations" }
                        table {
                            thead { tr {
                                th { "Observed" } th { "Item" } th { "Result" } th { "Source" } th { "Confidence" }
                            } }
                            tbody { id: "settlement-observations" }
                        }
                    }
                    section {
                        h2 { title: "Sanitized final-score evidence submitted by public-source or account-history agents. Evidence can drive deterministic paper settlement for supported markets.", "External result evidence" }
                        table {
                            thead { tr {
                                th { "Observed" } th { "Event" } th { "Score" } th { "Source" } th { "Used" }
                            } }
                            tbody { id: "external-result-evidence" }
                        }
                    }
                    section {
                        h2 { title: "Audit trail of result-review checks and recommendations. These rows explain what source class was consulted and why a position remains unresolved.", "Settlement lookup attempts" }
                        table {
                            thead { tr {
                                th { "Checked" } th { "Item" } th { "Recommendation" } th { "Source" } th { "State" }
                            } }
                            tbody { id: "settlement-lookup-attempts" }
                        }
                    }
                    section {
                        h2 { "Lookup due queue" }
                        table {
                            thead { tr {
                                th { "Item" } th { "Expected" } th { "Last lookup" } th { "Status" }
                            } }
                            tbody { id: "lookup-due-items" }
                        }
                    }
                    section {
                        h2 { "Strategies played" }
                        table {
                            thead { tr {
                                th { "Strategy" } th { "Played" } th { "Open" } th { "Awaiting" } th { "P/L" }
                            } }
                            tbody { id: "played" }
                        }
                    }
                    section {
                        h2 { "Recent plays" }
                        table {
                            thead { tr {
                                th { "Created" } th { "Type" } th { "Selection" } th { "Stake" } th { "Status" }
                            } }
                            tbody { id: "recent-plays" }
                        }
                    }
                    section {
                        h2 { "Performance" }
                        table {
                            thead { tr {
                                th { "Sport" } th { "Played" } th { "Open" } th { "Due" } th { "P/L" } th { "Hit rate" }
                            } }
                            tbody { id: "performance" }
                        }
                    }
                    section {
                        h2 { title: "Europe/Copenhagen local-day paper performance for singles and coupons. This answers today without database access.", "Today" }
                        div { class: "operator-note", id: "today-window", "Loading today..." }
                        table {
                            thead { tr {
                                th { "Scope" } th { "Played" } th { "Settled" } th { "Open" } th { "P/L" } th { "Hit rate" }
                            } }
                            tbody { id: "today-performance" }
                        }
                    }
                    section {
                        h2 { title: "Previous Europe/Copenhagen local-day paper performance for singles and coupons.", "Yesterday" }
                        div { class: "operator-note", id: "yesterday-window", "Loading yesterday..." }
                        table {
                            thead { tr {
                                th { "Scope" } th { "Played" } th { "Settled" } th { "Open" } th { "P/L" } th { "Hit rate" }
                            } }
                            tbody { id: "yesterday-performance" }
                        }
                    }
                    section {
                        h2 { title: "Load any Europe/Copenhagen local-day paper performance slice by date.", "Daily lookup" }
                        div { class: "inline-controls",
                            input { id: "daily-performance-date", r#type: "date", title: "Europe/Copenhagen local date to inspect." }
                            button { id: "load-daily-performance", title: "Load paper performance for the selected local date.", "Load date" }
                        }
                        div { class: "operator-note", id: "daily-performance-window", "Select a date to load a daily report." }
                        table {
                            thead { tr {
                                th { "Scope" } th { "Played" } th { "Settled" } th { "Open" } th { "P/L" } th { "Hit rate" }
                            } }
                            tbody { id: "daily-performance" }
                        }
                        h2 { title: "Recent paper placements in the selected local-day report.", "Daily placements" }
                        table {
                            thead { tr {
                                th { "Created" } th { "Type" } th { "Selection" } th { "Stake" } th { "Result check" } th { "Status" }
                            } }
                            tbody { id: "daily-performance-recent" }
                        }
                    }
                    section {
                        h2 { "Risk flag performance" }
                        table {
                            thead { tr {
                                th { "Flag" } th { "Played" } th { "Open" } th { "P/L" } th { "Hit rate" }
                            } }
                            tbody { id: "risk-performance" }
                        }
                    }
                    section {
                        h2 { "Performance history" }
                        table {
                            thead { tr {
                                th { "Recorded" } th { "Source" } th { "Open exposure" } th { "Due" } th { "P/L" } th { "Capacity" }
                            } }
                            tbody { id: "performance-history" }
                        }
                    }
                    section {
                        h2 { "Opportunity intake" }
                        table {
                            thead { tr {
                                th { "State" } th { "Count" } th { "Avg score" } th { "Avg confidence" }
                            } }
                            tbody { id: "opportunity-intake" }
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
                        h2 { "Provider coupon rules" }
                        table {
                            thead { tr {
                                th { "Sport" } th { "Market" } th { "Accumulator" } th { "Scope" } th { "Observed" }
                            } }
                            tbody { id: "coupon-rules" }
                        }
                    }
                    section {
                        h2 { "Odds movement" }
                        table {
                            thead { tr {
                                th { "Selection" } th { "Previous" } th { "Current" } th { "Move" } th { "Observed" }
                            } }
                            tbody { id: "odds-movement" }
                        }
                    }
                    section {
                        h2 { "Sports intelligence" }
                        table {
                            thead { tr {
                                th { "Sport" } th { "Events" } th { "Features" } th { "Confidence" } th { "Missing" }
                            } }
                            tbody { id: "intelligence" }
                        }
                    }
                    section {
                        h2 { "Ingestion runs" }
                        table {
                            thead { tr {
                                th { "Completed" } th { "Source" } th { "Status" } th { "Sports" } th { "Events" }
                            } }
                            tbody { id: "ingestion-runs" }
                        }
                    }
                    section {
                        h2 { "Audit events" }
                        table {
                            thead { tr {
                                th { "Created" } th { "Type" } th { "Details" }
                            } }
                            tbody { id: "audit-events" }
                        }
                    }
                    section {
                        h2 { "Strategy experiments" }
                        table {
                            thead { tr {
                                th { "Status" } th { "Variable" } th { "Change" } th { "Evidence" } th {}
                            } }
                            tbody { id: "experiments" }
                        }
                    }
                    section {
                        h2 { title: "Latest scheduled or manual Hermes-safe cycle. Replay refresh updates evidence only; it does not place paper bets, change experiment status, or promote a baseline.", "Hermes cycle" }
                        table {
                            thead { tr {
                                th { "Completed" } th { "Trigger" } th { "Reflection" } th { "Replay refresh" } th { "Safety" }
                            } }
                            tbody { id: "hermes-cycle" }
                        }
                    }
                    section {
                        h2 { title: "Hermes promotion gates for active strategy experiments. A row must clear every blocker before operator promotion review is allowed.", "Hermes promotion gates" }
                        table {
                            thead { tr {
                                th { "Experiment" } th { "Eligible" } th { "Policy evidence" } th { "Blockers" } th { "Recommendation" }
                            } }
                            tbody { id: "hermes-promotion-gates" }
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
input {
  min-height: 34px; border: 1px solid #aeb8c6; border-radius: 6px;
  padding: 0 10px; font: inherit; background: #ffffff; color: #17202a;
}
.inline-controls { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin-bottom: 10px; }
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
.operator-note { border-top: 1px solid #e6ebf2; border-bottom: 1px solid #e6ebf2; padding: 10px 0; margin-bottom: 10px; font-size: 13px; }
.operator-note code { background: #f1f5f9; border: 1px solid #d9dee7; border-radius: 4px; padding: 2px 5px; word-break: break-word; }
.muted { color: #596678; }
.actions { display: flex; gap: 6px; flex-wrap: wrap; }
.actions button { min-height: 28px; padding: 0 8px; font-size: 12px; }
.source-links { display: flex; flex-direction: column; gap: 3px; margin-top: 6px; }
.settlement-selected { background: #eef6ff; outline: 2px solid #1f6feb; outline-offset: -2px; }
.actions button.selected { background: #1f6feb; color: #ffffff; border-color: #1f6feb; }
.danger { color: #9f1239; }
.ok { color: #166534; }
pre { white-space: pre-wrap; overflow: auto; max-height: 420px; background: #0f172a; color: #e2e8f0; padding: 12px; border-radius: 6px; }
"#;

const SCRIPT: &str = r#"
const $ = (id) => document.getElementById(id);
const appBase = document.body.dataset.basePath || "";
const api = (path) => `${appBase}${path}`;
const money = (value) => Number(value || 0).toFixed(2);
const maybeMoney = (value) => value === null || value === undefined ? "-" : money(value);
const pct = (value) => value === null || value === undefined ? "-" : `${(Number(value) * 100).toFixed(1)}%`;
const num = (value) => value === null || value === undefined ? "-" : Number(value).toFixed(3);
const mins = (value) => value === null || value === undefined ? "-" : `${Math.round(Number(value) / 60)}m`;
const durationMins = (value) => {
  if (value === null || value === undefined) return "";
  const total = Math.max(0, Number(value));
  const days = Math.floor(total / 1440);
  const hours = Math.floor((total % 1440) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h`;
  return `${Math.round(total)}m`;
};
const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
}[ch]));
const hostLabel = (url) => {
  try { return new URL(url).hostname; } catch (_) { return url || ""; }
};
const renderExternalResultLink = (link) => {
  if (!link || !link.source_url) return "";
  const source = link.source_key || "external_result";
  const browser = link.requires_browser_automation ? " browser evidence" : " direct check";
  return `<div><a class="muted" href="${esc(link.source_url)}" target="_blank" rel="noreferrer">${esc(source)} (${esc(hostLabel(link.source_url))})</a><br><span class="label">${esc(browser)}</span></div>`;
};
const renderExternalResultLinks = (item) => {
  const links = item.item_type === "coupon"
    ? (Array.isArray(item.external_result_links) ? item.external_result_links : [])
    : (Array.isArray(item.external_result_links) && item.external_result_links.length
      ? item.external_result_links
      : (item.external_result_link ? [item.external_result_link] : []));
  const rendered = links.map(renderExternalResultLink).filter(Boolean);
  if (!rendered.length) return "";
  return `<div class="source-links">${rendered.join("")}</div>`;
};
const openSettlementStatuses = ["open", "awaiting_result", "unresolved", "postponed"];
const settlementActions = [
  ["won", "Won"],
  ["lost", "Lost"],
  ["void", "Void"],
  ["pushed", "Push"],
  ["refunded", "Refund"],
  ["cancelled", "Cancel"],
  ["postponed", "Postpone"]
];
const settlementButtons = (attribute, id, disabled) => settlementActions.map(([result, label]) =>
  `<button ${attribute}="${esc(id || "")}" data-result="${result}" ${disabled || !id ? "disabled" : ""}>${label}</button>`
).join("");
const reviewSettlementButtons = (attribute, id, disabled, sourceKey) => settlementActions.map(([result, label]) =>
  `<button ${attribute}="${esc(id || "")}" data-result="${result}" data-source-key="${esc(sourceKey || "")}" ${disabled || !id ? "disabled" : ""}>${label}</button>`
).join("");
let currentSettlementSourceKey = "danskespil_account_history";
const pendingSettlementReviews = new Map();
const pendingSettlementKey = (type, id) => `${type}:${id}`;
const updatePendingSettlementUi = () => {
  $("commit-settlements").disabled = pendingSettlementReviews.size === 0;
  $("commit-settlements").textContent = pendingSettlementReviews.size
    ? `Commit selected settlements (${pendingSettlementReviews.size})`
    : "Commit selected settlements";
};
const settlementSourceKey = (policy) => {
  const preferred = ((policy || {}).items || [])[0] || {};
  return preferred.source_key || currentSettlementSourceKey;
};
const json = (url, options = {}) => fetch(url, {
  headers: { "content-type": "application/json" },
  ...options
}).then((r) => {
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
  return r.json();
});
function renderRows(items) {
  $("candidates").innerHTML = items.map((item) => {
    const movement = item.feature_snapshot && item.feature_snapshot.odds_movement
      ? item.feature_snapshot.odds_movement
      : null;
    const movementBand = movement && movement.classification
      ? movement.classification.movement_band || ""
      : "";
    const movementLabel = movement
      ? `${movement.decimal_odds_delta >= 0 ? "+" : ""}${Number(movement.decimal_odds_delta || 0).toFixed(2)} ${movement.direction || ""}${movementBand ? ` / ${movementBand}` : ""}`
      : "no prior";
    const movementClass = movement && movement.direction === "up" ? "ok" : movement && movement.direction === "down" ? "danger" : "muted";
    return `
      <tr>
        <td><span class="pill">${esc(item.sport_key)}</span></td>
        <td>${esc(item.event_name)}<br><span class="label">${esc(item.competition)}</span></td>
        <td>${esc(item.market_name)}<br><span class="label">${esc(item.market_kind)}</span></td>
        <td>${esc(item.outcome_name)}</td>
        <td>${item.decimal_odds ?? ""}<br><span class="muted">imp ${pct(item.implied_probability)}</span><br><span class="${movementClass}">${esc(movementLabel)}</span></td>
        <td>${num(item.score)}<br><span class="muted">conf ${pct(item.confidence)}</span></td>
        <td><button data-candidate="${item.id}" ${item.status === "rejected" ? "disabled" : ""}>${item.status === "rejected" ? "Rejected" : "Paper"}</button></td>
      </tr>
    `;
  }).join("");
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
function renderStrategyDecisions(items) {
  const selected = items.filter((item) => item.decision === "selected").length;
  const rejected = items.filter((item) => item.decision === "rejected").length;
  $("strategy-selected").textContent = String(selected);
  $("strategy-rejected").textContent = String(rejected);
  $("strategy-decisions").innerHTML = items.map((item) => {
    const candidate = item.candidate || {};
    const reasons = Array.isArray(item.rejection_reasons) && item.rejection_reasons.length
      ? item.rejection_reasons.join(", ")
      : "-";
    return `
      <tr>
        <td><span class="pill">${esc(item.decision)}</span><br><span class="label">v${esc(item.strategy_version)}</span></td>
        <td>${esc(candidate.event_name || "")}<br><span class="label">${esc(candidate.market_name || "")} / ${esc(candidate.outcome_name || "")}</span></td>
        <td>${candidate.decimal_odds ?? "-"}</td>
        <td>${num(item.score)}<br><span class="muted">conf ${pct(item.confidence)}</span></td>
        <td>${esc(reasons)}</td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("strategy-decisions").innerHTML = `<tr><td colspan="5" class="muted">No strategy decisions yet. Run a scan.</td></tr>`;
  }
}
function renderCoupons(items) {
  $("coupons").innerHTML = items.map((item) => {
    const evidence = item.provider_rule_evidence || {};
    const legs = item.legs || [];
    const legLabels = legs.map((leg) => {
      const candidate = leg.payload && leg.payload.candidate ? leg.payload.candidate : {};
      return `${candidate.event_name || leg.candidate_id || ""} / ${candidate.outcome_name || ""}`;
    }).join("<br>");
    const ruleText = [
      evidence.sport_key ? `sport ${evidence.sport_key}` : null,
      evidence.same_sport_validation ? "same sport" : null,
      evidence.distinct_event_validation ? "distinct events" : null
    ].filter(Boolean).join(", ");
    return `
      <tr>
        <td><span class="pill">${esc(item.coupon_type)}</span><br><span class="label">${esc(item.status)}</span></td>
        <td>${legLabels || esc(item.leg_count)}</td>
        <td>${num(item.combined_decimal_odds)}</td>
        <td>${num(item.score)}<br><span class="muted">conf ${pct(item.confidence)}</span></td>
        <td>${esc(ruleText || "not verified")}</td>
        <td><button data-coupon="${item.id}" ${item.status === "rejected" ? "disabled" : ""}>Paper coupon</button></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("coupons").innerHTML = `<tr><td colspan="6" class="muted">No multi-leg coupon candidates yet. They remain disabled unless the active baseline enables them.</td></tr>`;
  }
  document.querySelectorAll("[data-coupon]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/coupons/simulate"), {
        method: "POST",
        body: JSON.stringify({ coupon_id: button.dataset.coupon })
      });
      await load();
    });
  });
}
function renderSimulatedCoupons(items) {
  const couponLabel = (item) => {
    const coupon = item.payload && item.payload.coupon ? item.payload.coupon : {};
    const legs = coupon.legs || item.legs || [];
    const labels = legs.map((leg) => {
      const candidate = leg.payload && leg.payload.candidate ? leg.payload.candidate : {};
      return `${esc(candidate.event_name || leg.candidate_id || "")} / ${esc(candidate.outcome_name || "")}`;
    }).filter(Boolean);
    return `${esc(coupon.coupon_type || "coupon")} (${labels.length || coupon.leg_count || 0})<br><span class="label">${labels.join("<br>")}</span>`;
  };
  $("simulated-coupons").innerHTML = items.map((item) => {
    const canSettle = openSettlementStatuses.includes(item.status);
    return `
      <tr>
        <td>${esc(item.created_at)}</td>
        <td>${couponLabel(item)}<br><span class="label">${esc(item.strategy_id || "")}</span></td>
        <td>${money(item.hypothetical_stake)}<br><span class="muted">@ ${item.observed_combined_decimal_odds ?? "-"}</span></td>
        <td>${esc(item.expected_result_check_after || "-")}<br><span class="muted">${esc(item.latest_event_start_time || "")}</span></td>
        <td>${esc(item.status)}</td>
        <td>${item.profit_loss === null || item.profit_loss === undefined ? "-" : money(item.profit_loss)}</td>
        <td>
          <div class="actions">
            ${settlementButtons("data-coupon-settle", item.id, !canSettle)}
          </div>
        </td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("simulated-coupons").innerHTML = `<tr><td colspan="7" class="muted">No paper coupons have been simulated yet.</td></tr>`;
  }
  document.querySelectorAll("[data-coupon-settle]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/coupons/settle"), {
        method: "POST",
        body: JSON.stringify({
          coupon_id: button.dataset.couponSettle,
          result: button.dataset.result,
          source: currentSettlementSourceKey,
          confidence: 1
        })
      });
      await load();
    });
  });
}
function renderLedger(items) {
  const candidateLabel = (item) => {
    const candidate = item.payload && item.payload.candidate ? item.payload.candidate : {};
    const eventName = item.event_name || candidate.event_name || item.candidate_id || "";
    const outcomeName = item.outcome_name || candidate.outcome_name || "";
    return `${eventName} / ${outcomeName}`;
  };
  $("ledger").innerHTML = items.map((item) => {
    const canSettle = openSettlementStatuses.includes(item.status);
    const detail = [item.sport_key, item.competition, item.market_name].filter(Boolean).join(" / ");
    return `
    <tr>
      <td>${esc(item.created_at)}</td>
      <td>${esc(candidateLabel(item))}<br><span class="label">${esc(detail || item.strategy_id || "")}</span></td>
      <td>${money(item.hypothetical_stake)}<br><span class="muted">@ ${item.observed_decimal_odds ?? "-"}</span></td>
      <td>${esc(item.expected_result_check_after || "-")}<br><span class="muted">${esc(item.event_start_time || "")}</span></td>
      <td>${esc(item.status)}</td>
      <td>${item.profit_loss === null || item.profit_loss === undefined ? "-" : money(item.profit_loss)}</td>
      <td>
        <div class="actions">
          ${settlementButtons("data-settle", item.id, !canSettle)}
        </div>
      </td>
    </tr>
    `;
  }).join("");
  document.querySelectorAll("[data-settle]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/ledger/settle"), {
        method: "POST",
        body: JSON.stringify({
          bet_id: button.dataset.settle,
          result: button.dataset.result,
          source: currentSettlementSourceKey,
          confidence: 1
        })
      });
      await load();
    });
  });
}
function renderSettlementReview(summary) {
  const items = summary.items || [];
  $("settlement-review").innerHTML = items.map((item) => {
    const isCoupon = item.item_type === "coupon";
    const legs = item.legs || [];
    const firstLeg = legs[0] || {};
    const policy = item.settlement_source_policy || summary.settlement_source_policy || {};
    const preferredSource = (policy.items || [])[0] || {};
    const selection = isCoupon
      ? `${item.coupon_type || "coupon"} (${item.leg_count || legs.length})`
      : `${item.event_name || item.bet_id || ""}`;
    const detail = isCoupon
      ? legs.map((leg) => `${esc(leg.event_name || leg.candidate_id || "")} / ${esc(leg.outcome_name || "")}`).join("<br>")
      : esc(`${item.market_name || ""} / ${item.outcome_name || ""}`);
    const state = isCoupon
      ? `${legs.filter((leg) => leg.event_resulted || leg.event_settled).length}/${legs.length} legs resulted`
      : `resulted ${item.event_resulted} / settled ${item.event_settled}`;
    const status = isCoupon ? item.coupon_status : item.bet_status;
    const canSettle = openSettlementStatuses.includes(status);
    const actionAttribute = isCoupon ? "data-review-coupon-settle" : "data-review-settle";
    const actionId = isCoupon ? item.coupon_simulation_id : item.bet_id;
    const pendingKey = pendingSettlementKey(isCoupon ? "coupon" : "single", actionId || "");
    const pending = pendingSettlementReviews.get(pendingKey);
    const lookupLabel = item.last_lookup_at
      ? `last lookup ${item.last_lookup_at}`
      : "lookup not recorded";
    const lookupClass = item.lookup_stale ? "danger" : "ok";
    const overdueLabel = item.overdue_minutes > 0 ? `overdue ${durationMins(item.overdue_minutes)}` : "";
    const sourceLabel = item.recommended_source_key || preferredSource.source_key || "-";
    const sourceOptions = Array.isArray(item.recommended_source_keys) && item.recommended_source_keys.length
      ? item.recommended_source_keys.join(", ")
      : sourceLabel;
    return `
      <tr data-settlement-row="${esc(pendingKey)}" class="${pending ? "settlement-selected" : ""}">
        <td><span class="pill">${esc(item.item_type || "single")}</span> ${esc(selection)}<br><span class="label">${detail}</span></td>
        <td>${esc(item.expected_result_check_after || "-")}<br><span class="muted">${esc(item.sport_key || firstLeg.sport_key || "")}</span><br><span class="${lookupClass}">${esc(lookupLabel)}</span>${overdueLabel ? `<br><span class="danger">${esc(overdueLabel)}</span>` : ""}</td>
        <td>${esc(item.event_status || firstLeg.event_status || "-")}<br><span class="muted">${esc(state)}</span></td>
        <td>
          <span class="pill">${esc(item.recommendation || "await_more_evidence")}</span>
          <br><span class="muted">source: ${esc(sourceOptions)}</span>
          ${renderExternalResultLinks(item)}
          <div class="actions">${reviewSettlementButtons(actionAttribute, actionId, !canSettle, sourceLabel)}</div>
          ${pending ? `<span class="label">selected: ${esc(pending.result)} via ${esc(pending.source)}</span>` : ""}
        </td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("settlement-review").innerHTML = `<tr><td colspan="4" class="muted">No awaiting-result paper bets or coupons need review.</td></tr>`;
  }
  document.querySelectorAll("[data-review-settle]").forEach((button) => {
    button.addEventListener("click", async () => {
      const key = pendingSettlementKey("single", button.dataset.reviewSettle || "");
      pendingSettlementReviews.set(key, {
        type: "single",
        id: button.dataset.reviewSettle,
        result: button.dataset.result,
        source: button.dataset.sourceKey || settlementSourceKey(summary.settlement_source_policy)
      });
      document.querySelectorAll(`[data-review-settle="${CSS.escape(button.dataset.reviewSettle || "")}"]`).forEach((item) => {
        item.classList.toggle("selected", item.dataset.result === button.dataset.result);
      });
      const row = document.querySelector(`[data-settlement-row="${CSS.escape(key)}"]`);
      if (row) row.classList.add("settlement-selected");
      updatePendingSettlementUi();
    });
  });
  document.querySelectorAll("[data-review-coupon-settle]").forEach((button) => {
    button.addEventListener("click", async () => {
      const key = pendingSettlementKey("coupon", button.dataset.reviewCouponSettle || "");
      pendingSettlementReviews.set(key, {
        type: "coupon",
        id: button.dataset.reviewCouponSettle,
        result: button.dataset.result,
        source: button.dataset.sourceKey || settlementSourceKey(summary.settlement_source_policy)
      });
      document.querySelectorAll(`[data-review-coupon-settle="${CSS.escape(button.dataset.reviewCouponSettle || "")}"]`).forEach((item) => {
        item.classList.toggle("selected", item.dataset.result === button.dataset.result);
      });
      const row = document.querySelector(`[data-settlement-row="${CSS.escape(key)}"]`);
      if (row) row.classList.add("settlement-selected");
      updatePendingSettlementUi();
    });
  });
  updatePendingSettlementUi();
}
function renderResultAgentQueue(queue) {
  const items = queue.items || [];
  $("result-agent-tasks").textContent = `${queue.task_count ?? items.length} / ${money(queue.task_exposure)}`;
  $("result-agent-tasks").className = Number(queue.task_count || items.length || 0) > 0 ? "value danger" : "value ok";
  renderResultAgentCycle(queue.latest_cycle || null);
  renderResultAgentCycleHistory(queue.recent_cycles || []);
  $("result-agent-queue").innerHTML = items.map((item) => {
    const selection = item.selection || {};
    const ids = item.ids || {};
    const links = item.source_links || [];
    const linkText = links.length
      ? links.map(renderExternalResultLink).join("")
      : `<span class="muted">no configured result link</span>`;
    const terms = (item.search_terms || []).slice(0, 4).map(esc).join("<br>");
    const eventNames = (selection.event_names || []).length
      ? selection.event_names.map(esc).join("<br>")
      : selection.event_name || ids.bet_id || ids.coupon_simulation_id || "-";
    const overdueText = item.overdue_minutes === null || item.overdue_minutes === undefined
      ? ""
      : `<br><span class="danger">${esc(durationMins(item.overdue_minutes))} overdue</span>`;
    const taskTitle = `${item.agent_action || "result-agent task"} Evidence should be posted to ${item.evidence_endpoint || "/api/settlement/external-evidence"}.`;
    const sourceTitle = links.length
      ? "Configured result links are ready for direct worker checks or browser-backed result-agent probes."
      : "No configured result link yet. A source-discovery/account-history agent should find evidence automatically; operators should not need to paste URLs.";
    return `
      <tr title="${esc(taskTitle)}">
        <td><span class="pill">${esc(item.task_kind || "result_task")}</span><br><span class="label">${esc(item.automation_status || "")}</span>${overdueText}<br><span class="muted">stake ${money(item.hypothetical_stake)} / priority ${num(item.priority_score)}</span></td>
        <td>${Array.isArray(selection.event_names) && selection.event_names.length ? eventNames : esc(eventNames)}<br><span class="label">${esc([selection.sport_key, selection.competition, selection.market_name, selection.outcome_name].filter(Boolean).join(" / "))}</span></td>
        <td title="${esc(sourceTitle)}">${linkText}${terms ? `<br><span class="label">terms</span><br><span class="muted">${terms}</span>` : ""}</td>
        <td>${esc(item.agent_action || "")}<br><span class="label">evidence: ${esc(item.evidence_endpoint || "")}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("result-agent-queue").innerHTML = `<tr><td colspan="4" class="muted">No result-agent tasks are due.</td></tr>`;
  }
}
function renderResultAgentCycle(cycle) {
  if (!cycle || !cycle.details) {
    $("result-agent-cycle").innerHTML = `<tr><td colspan="5" class="muted">No result-agent cycle has completed yet.</td></tr>`;
    return;
  }
  const details = cycle.details || {};
  $("result-agent-cycle").innerHTML = `
    <tr>
      <td>${esc(cycle.created_at || "-")}<br><span class="label">${esc(cycle.id || "")}</span></td>
      <td>${esc(details.queued_task_count ?? 0)}<br><span class="muted">${money(details.queued_task_exposure)}</span></td>
      <td>${esc(details.selected_task_count ?? 0)} / ${esc(details.cycle_limit ?? "-")}<br><span class="muted">${money(details.selected_task_exposure)}</span><br><span class="label">max priority ${num(details.max_selected_priority)}</span></td>
      <td>${esc(details.task_attempted_count ?? 0)}<br><span class="muted">${money(details.task_attempted_exposure)}</span><br><span class="label">skipped ${money(details.task_skipped_exposure)}</span></td>
      <td>settled ${esc(details.settled_count ?? 0)}<br><span class="label">results ${esc(details.attempted_count ?? 0)}, skipped ${esc(details.skipped_count ?? 0)}</span></td>
    </tr>
  `;
}
function renderResultAgentCycleHistory(cycles) {
  const items = Array.isArray(cycles) ? cycles : [];
  if (!items.length) {
    $("result-agent-cycle-history").innerHTML = `<tr><td colspan="6" class="muted">No result-agent cycle history has been recorded yet.</td></tr>`;
    return;
  }
  $("result-agent-cycle-history").innerHTML = items.map((cycle) => {
    const details = cycle.details || {};
    return `
      <tr>
        <td>${esc(cycle.created_at || "-")}<br><span class="label">${esc(cycle.id || "")}</span></td>
        <td>${esc(details.queued_task_count ?? 0)}<br><span class="muted">${money(details.queued_task_exposure)}</span></td>
        <td>${esc(details.selected_task_count ?? 0)} / ${esc(details.cycle_limit ?? "-")}<br><span class="muted">${money(details.selected_task_exposure)}</span></td>
        <td>${esc(details.task_attempted_count ?? 0)}<br><span class="muted">${money(details.task_attempted_exposure)}</span></td>
        <td>${esc(details.skipped_count ?? 0)}<br><span class="muted">${money(details.task_skipped_exposure)}</span></td>
        <td>${esc(details.settled_count ?? 0)}<br><span class="label">results ${esc(details.attempted_count ?? 0)}</span></td>
      </tr>
    `;
  }).join("");
}
function renderAccountHistoryRequests(requests) {
  const items = requests.items || [];
  const runbook = requests.local_agent_runbook || {};
  const agent = requests.danskespil_account_agent || {};
  const dryRunCommand = runbook.make_dry_run_target || runbook.dry_run_command || "";
  $("account-history-agent-runbook").innerHTML = `
    <div><span class="label">Local account-history agent</span></div>
    <div>${esc(items.length)} request${items.length === 1 ? "" : "s"} / ${money(requests.request_exposure)} pending. Run locally with an operator-controlled browser session; the cluster cannot access account history.</div>
    <div class="muted">First port-forward: <code>${esc(runbook.port_forward_command || "-")}</code></div>
    <div class="muted">Then dry-run: <code>${esc(dryRunCommand || "-")}</code></div>
    <div class="${agent.available ? "ok" : "muted"}">${esc(agent.available ? "Local credential env vars are present in this process context." : "Credential values are not exposed here; sign in only in the local browser session.")}</div>
  `;
  $("account-history-requests").innerHTML = items.map((item) => {
    const selection = item.selection || {};
    const ids = item.ids || {};
    const eventNames = (selection.event_names || []).length
      ? selection.event_names.map(esc).join("<br>")
      : esc(selection.event_name || ids.bet_id || ids.coupon_simulation_id || "-");
    const overdueText = item.overdue_minutes === null || item.overdue_minutes === undefined
      ? ""
      : `<br><span class="danger">${esc(durationMins(item.overdue_minutes))} overdue</span>`;
    const template = item.evidence_template || {};
    const contract = [
      `source ${template.source_key || item.source_key || "-"}`,
      item.evidence_endpoint || "",
      template.settle === false ? "settle=false first" : ""
    ].filter(Boolean).map(esc).join("<br>");
    return `
      <tr title="${esc("Use a local operator browser session. Do not submit bets or store credentials, cookies, browser storage, full account pages, payment data, Spil-ID, or MitID payloads.")}">
        <td><span class="pill">${esc(item.request_kind || "account_history_request")}</span><br><span class="label">${esc(item.recommendation || "")}</span>${overdueText}<br><span class="muted">stake ${money(item.hypothetical_stake)} / priority ${num(item.priority_score)}</span></td>
        <td>${Array.isArray(selection.event_names) && selection.event_names.length ? eventNames : eventNames}<br><span class="label">${esc([selection.sport_key, selection.competition, selection.market_name, selection.outcome_name].filter(Boolean).join(" / "))}</span></td>
        <td>${esc(item.expected_truth || "")}<br><span class="label">${esc(item.lookup_stale ? "lookup stale" : "recent lookup")}</span></td>
        <td>${contract}<br><span class="label">paper-only sanitized facts</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    const readiness = agent.available ? "No account-history requests are due." : "No account-history requests are due, or no local account-agent context is configured.";
    $("account-history-requests").innerHTML = `<tr><td colspan="4" class="muted">${esc(readiness)}</td></tr>`;
  }
}
function renderSettlementSources(sources) {
  const items = sources.items || [];
  currentSettlementSourceKey = settlementSourceKey(sources);
  $("settlement-sources").innerHTML = items.map((item) => {
    const payload = item.payload || {};
    return `
      <tr>
        <td>${esc(payload.priority ?? "-")}</td>
        <td><span class="pill">${esc(item.source_key)}</span><br><span class="label">${esc(item.source_type)}</span><br>${esc(item.source_name)}</td>
        <td>${esc((item.sport_scope || []).join(", "))}</td>
        <td>${num(item.reliability)}</td>
        <td>${esc(item.notes || "")}<br><span class="muted">${esc(item.url_pattern || "")}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("settlement-sources").innerHTML = `<tr><td colspan="5" class="muted">No settlement-capable sources are configured.</td></tr>`;
  }
}
function renderExternalResultLinksTable(links) {
  const items = links.items || [];
  $("external-result-links").innerHTML = items.map((item) => {
    const sourceLink = item.source_url
      ? `<a class="muted" href="${esc(item.source_url)}" target="_blank" rel="noreferrer">${esc(hostLabel(item.source_url))}</a>`
      : "-";
    return `
      <tr>
        <td>${esc(item.updated_at || "-")}<br><span class="label">${esc(item.created_at || "")}</span></td>
        <td>${esc(item.event_name || "-")}<br><span class="label">${esc(item.id || "")}</span></td>
        <td><span class="pill">${esc(item.source_key || "-")}</span><br>${sourceLink}</td>
        <td><span class="label">home</span> ${esc((item.home_aliases || []).join(", ") || "-")}<br><span class="label">away</span> ${esc((item.away_aliases || []).join(", ") || "-")}</td>
        <td>${item.requires_browser_automation ? "browser evidence" : "direct check"}</td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("external-result-links").innerHTML = `<tr><td colspan="5" class="muted">No operator result links have been added.</td></tr>`;
  }
}
function renderEntityAliases(aliases) {
  const items = aliases.items || [];
  $("entity-aliases").innerHTML = items.map((item) => `
    <tr>
      <td>${esc(item.last_seen_at || "-")}<br><span class="label">${esc(item.first_seen_at || "")}</span></td>
      <td><span class="pill">${esc(item.entity_kind || "-")}</span> ${esc(item.canonical_name || "-")}<br><span class="label">${esc([item.sport_key || "all sports", item.gender_scope || "any gender"].join(" / "))}</span></td>
      <td>${esc(item.alias_name || "-")}<br><span class="muted">${esc(item.alias_key || "")}</span></td>
      <td>${esc(item.source_key || "-")}<br><span class="muted">${esc(item.external_id || "")}</span></td>
      <td>${num(item.confidence)}</td>
    </tr>
  `).join("");
  if (!items.length) {
    $("entity-aliases").innerHTML = `<tr><td colspan="5" class="muted">No aliases have been recorded yet.</td></tr>`;
  }
}
function evidenceEventNames(payload, fallbackEventName) {
  const fallback = String(fallbackEventName || "").trim();
  const names = Array.isArray(payload && payload.event_names)
    ? payload.event_names.map((value) => String(value || "").trim()).filter(Boolean)
    : [];
  const unique = [];
  const seen = new Set();
  names.forEach((name) => {
    const key = name.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      unique.push(name);
    }
  });
  if (!unique.length || (unique.length === 1 && unique[0] === fallback)) {
    return "";
  }
  return unique.map(esc).join("<br>");
}
function renderSettlementObservations(observations) {
  const items = observations.items || [];
  $("settlement-observations").innerHTML = items.map((item) => {
    const policy = item.source_policy || {};
    const sourceLabel = policy.source_name || item.source;
    const itemLabel = item.item_type === "coupon"
      ? `${item.coupon_type || "coupon"} (${item.leg_count || 0})`
      : `${item.event_name || item.simulated_bet_id || ""}`;
    const baseDetail = item.item_type === "coupon"
      ? item.strategy_id || ""
      : [item.market_name, item.outcome_name].filter(Boolean).join(" / ");
    const eventNames = evidenceEventNames(item.payload || {}, item.event_name);
    let detail = baseDetail ? `<span class="label">${esc(baseDetail)}</span>` : "";
    if (eventNames) {
      detail = `${detail ? `${detail}<br>` : ""}<span class="label">legs</span><br><span class="muted">${eventNames}</span>`;
    }
    return `
      <tr>
        <td>${esc(item.created_at || "-")}</td>
        <td><span class="pill">${esc(item.item_type)}</span> ${esc(itemLabel)}${detail ? `<br>${detail}` : ""}</td>
        <td>${esc(item.observed_result)}<br><span class="muted">${esc(item.status || "")}</span></td>
        <td>${esc(item.source)}<br><span class="muted">${esc(sourceLabel)}</span></td>
        <td>${pct(item.confidence)}</td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("settlement-observations").innerHTML = `<tr><td colspan="5" class="muted">No settlement observations have been recorded yet.</td></tr>`;
  }
}
function renderExternalResultEvidence(evidence) {
  const items = evidence.items || [];
  $("external-result-evidence").innerHTML = items.map((item) => {
    const payload = item.payload || {};
    const excerpt = payload.raw_text_excerpt
      ? String(payload.raw_text_excerpt).slice(0, 180)
      : "";
    const scoreAvailable = payload.score_available !== false;
    const resultLabel = scoreAvailable
      ? `${esc(item.home_score)} - ${esc(item.away_score)}`
      : esc(payload.settlement_result || payload.result_status_raw || "status-only");
    const eventNames = evidenceEventNames(payload, item.event_name);
    const sourceUrl = item.source_url
      ? `<br><a class="muted" href="${esc(item.source_url)}" target="_blank" rel="noreferrer">${esc(hostLabel(item.source_url))}</a>`
      : "";
    return `
      <tr>
        <td>${esc(item.created_at || "-")}</td>
        <td>${esc(item.event_name || "-")}<br><span class="label">${esc(item.home_name || "")} vs ${esc(item.away_name || "")}</span>${eventNames ? `<br><span class="label">legs</span><br><span class="muted">${eventNames}</span>` : ""}${excerpt ? `<br><span class="muted">${esc(excerpt)}</span>` : ""}</td>
        <td>${resultLabel}<br><span class="muted">${pct(item.confidence)}</span></td>
        <td>${esc(item.source_key || "-")}${sourceUrl}</td>
        <td><span class="${item.used_for_settlement ? "ok" : "muted"}">${item.used_for_settlement ? "settlement evidence" : "evidence only"}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("external-result-evidence").innerHTML = `<tr><td colspan="5" class="muted">No external result evidence has been recorded yet.</td></tr>`;
  }
}
function renderSettlementLookupAttempts(attempts) {
  const items = attempts.items || [];
  $("settlement-lookup-attempts").innerHTML = items.map((item) => {
    const itemLabel = item.item_type === "coupon"
      ? `${item.coupon_type || "coupon"} (${item.leg_count || 0})`
      : `${item.event_name || item.simulated_bet_id || ""}`;
    const detail = item.item_type === "coupon"
      ? item.simulated_coupon_id || ""
      : [item.market_name, item.outcome_name].filter(Boolean).join(" / ");
    const state = item.outcome_state || {};
    const stateLabel = [
      state.event_status ? `status ${state.event_status}` : null,
      state.event_resulted === true ? "resulted" : null,
      state.event_settled === true ? "settled" : null,
      state.latest_outcome_active === false ? "outcome inactive" : null
    ].filter(Boolean).join(", ");
    return `
      <tr>
        <td>${esc(item.created_at || "-")}</td>
        <td><span class="pill">${esc(item.item_type)}</span> ${esc(itemLabel)}<br><span class="label">${esc(detail)}</span></td>
        <td>${esc(item.recommendation)}</td>
        <td>${esc(item.source_key)}<br><span class="muted">no auto grade</span></td>
        <td>${esc(stateLabel || "awaiting evidence")}</td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("settlement-lookup-attempts").innerHTML = `<tr><td colspan="5" class="muted">No settlement lookup attempts have been recorded yet.</td></tr>`;
  }
}
function renderPlayed(summary) {
  const items = summary.by_strategy || [];
  $("played").innerHTML = items.map((item) => `
    <tr>
      <td>${esc(item.strategy_id)}<br><span class="label">singles ${esc(item.single_count || 0)} / coupons ${esc(item.coupon_count || 0)} / duplicates ${esc(item.duplicate_void_count || 0)}</span></td>
      <td>${esc(item.played_count)}</td>
      <td>${esc(item.open_count)}<br><span class="muted">${money(item.open_exposure)}</span></td>
      <td>${esc(item.awaiting_result_count)}<br><span class="muted">${money(item.awaiting_result_exposure)}</span></td>
      <td>${money(item.profit_loss)}</td>
    </tr>
  `).join("");
  if (!items.length) {
    $("played").innerHTML = `<tr><td colspan="5" class="muted">No paper strategy placements yet.</td></tr>`;
  }
  const riskFlags = summary.by_risk_flag || [];
  $("risk-performance").innerHTML = riskFlags.map((item) => `
    <tr>
      <td><span class="pill">${esc(item.risk_flag || "none")}</span><br><span class="label">singles ${esc(item.single_count || 0)} / coupons ${esc(item.coupon_count || 0)}</span></td>
      <td>${esc(item.played_count)}<br><span class="muted">${money(item.turnover)}</span></td>
      <td>${esc(item.open_count)}<br><span class="muted">${money(item.open_exposure)}</span><br><span class="label">awaiting ${esc(item.awaiting_result_count || 0)} / ${money(item.awaiting_result_exposure)}</span></td>
      <td>${money(item.profit_loss)}</td>
      <td>${pct(item.hit_rate)}<br><span class="muted">${esc(item.decided_count || 0)} decided</span></td>
    </tr>
  `).join("");
  if (!riskFlags.length) {
    $("risk-performance").innerHTML = `<tr><td colspan="5" class="muted">No paper risk-flag performance yet.</td></tr>`;
  }
  const recent = summary.recent || [];
  $("recent-plays").innerHTML = recent.map((item) => {
    const selection = item.item_type === "coupon"
      ? `${item.market_name || "coupon"} / ${item.outcome_name || ""}`
      : `${item.event_name || item.id || ""} / ${item.outcome_name || ""}`;
    const context = [item.sport_key, item.competition, item.market_kind].filter(Boolean).join(" / ");
    return `
      <tr>
        <td>${esc(item.created_at || "-")}</td>
        <td><span class="pill">${esc(item.item_type || "single")}</span><br><span class="label">${esc(item.strategy_id || "")}</span></td>
        <td>${esc(selection)}<br><span class="label">${esc(context)}</span></td>
        <td>${money(item.hypothetical_stake)}<br><span class="muted">@ ${esc(item.observed_decimal_odds ?? "-")}</span></td>
        <td>${esc(item.status || "-")}<br><span class="muted">score ${num(item.score)} / conf ${pct(item.confidence)}</span></td>
      </tr>
    `;
  }).join("");
  if (!recent.length) {
    $("recent-plays").innerHTML = `<tr><td colspan="5" class="muted">No recent paper plays yet.</td></tr>`;
  }
}
function renderPerformance(report) {
  const capacity = report.placement_capacity || {};
  const settlement = report.settlement_work || {};
  const lookup = settlement.lookup_cadence || {};
  $("next-capacity").textContent = `${capacity.next_scan_capacity ?? 0}/${capacity.per_scan_limit ?? 0}`;
  $("next-capacity").className = capacity.blocked ? "value danger" : "value ok";
  $("due-review").textContent = `${settlement.due_total || 0} / ${money(settlement.due_exposure)}`;
  $("due-review").className = Number(settlement.due_total || 0) > 0 ? "value danger" : "value ok";
  $("lookup-due").textContent = `${lookup.due_without_recent_lookup_count ?? 0}/${lookup.due_lookup_item_count ?? 0} / ${money(lookup.due_without_recent_lookup_exposure)}`;
  $("lookup-due").className = Number(lookup.due_without_recent_lookup_count || 0) > 0 ? "value danger" : "value ok";

  $("performance").innerHTML = (report.by_sport || []).map((item) => `
    <tr>
      <td><span class="pill">${esc(item.sport_key)}</span><br><span class="label">avg @ ${item.average_odds ? num(item.average_odds) : "-"}</span></td>
      <td>${esc(item.played_count)}<br><span class="muted">${money(item.turnover)}</span></td>
      <td>${esc(item.open_count)}<br><span class="muted">${money(item.open_exposure)}</span><br><span class="label">awaiting ${esc(item.awaiting_result_count || 0)} / ${money(item.awaiting_result_exposure)}</span></td>
      <td>${esc(item.due_count || 0)}<br><span class="muted">${money(item.due_exposure)}</span></td>
      <td>${money(item.profit_loss)}</td>
      <td>${pct(item.hit_rate)}</td>
    </tr>
  `).join("");
  if (!(report.by_sport || []).length) {
    $("performance").innerHTML = `<tr><td colspan="6" class="muted">No paper performance yet.</td></tr>`;
  }
  if (lookup.next_lookup_due_at) {
    $("reasoning").textContent = `${$("reasoning").textContent}\n\nSettlement lookup cadence:\n${JSON.stringify(lookup, null, 2)}`;
  }
  const lookupDueItems = settlement.lookup_due_items || [];
  $("lookup-due-items").innerHTML = lookupDueItems.map((item) => {
    const label = item.item_type === "coupon"
      ? `${item.coupon_type || "coupon"} (${item.leg_count || 0})`
      : `${item.event_name || item.id || ""}`;
    const detail = item.item_type === "coupon"
      ? item.id
      : [item.market_name, item.outcome_name].filter(Boolean).join(" / ");
    return `
      <tr>
        <td><span class="pill">${esc(item.item_type)}</span> ${esc(label)}<br><span class="label">${esc(detail || "")}</span></td>
        <td>${esc(item.expected_result_check_after || "-")}<br><span class="muted">${esc(item.sport_key || "")} / ${money(item.hypothetical_stake)}</span></td>
        <td>${esc(item.last_lookup_at || "never")}</td>
        <td>${esc(item.status || "-")}</td>
      </tr>
    `;
  }).join("");
  if (!lookupDueItems.length) {
    $("lookup-due-items").innerHTML = `<tr><td colspan="4" class="muted">No due paper positions are missing a fresh lookup.</td></tr>`;
  }

  const intake = report.opportunity_intake || {};
  $("opportunity-intake").innerHTML = (intake.latest_candidate_status || []).map((item) => `
    <tr>
      <td><span class="pill">${esc(item.status)}</span></td>
      <td>${esc(item.count)}</td>
      <td>${num(item.average_score)}</td>
      <td>${pct(item.average_confidence)}</td>
    </tr>
  `).join("");
  if (!(intake.latest_candidate_status || []).length) {
    $("opportunity-intake").innerHTML = `<tr><td colspan="4" class="muted">No latest candidate intake yet. Run a scan.</td></tr>`;
  }
}
function renderTodayPerformance(report) {
  renderDailyPerformance(report, "today-window", "today-performance", "today");
}
function renderYesterdayPerformance(report) {
  renderDailyPerformance(report, "yesterday-window", "yesterday-performance", "yesterday");
}
function renderSelectedDailyPerformance(report) {
  renderDailyPerformance(report, "daily-performance-window", "daily-performance", report.local_date || "selected date");
  renderDailyPerformanceRecent(report.recent || []);
}
function renderDailyPerformance(report, windowId, tableId, label) {
  const summary = report.summary || {};
  const observations = (report.settlement_observations || {}).items || [];
  const observationLabel = observations.length
    ? observations.map((item) => `${item.observed_result}: ${item.count}`).join(" / ")
    : `no settlement observations ${label}`;
  $(windowId).textContent = `${report.local_date || "-"} (${report.timezone || "local"}), ${report.window?.start || "-"} to ${report.window?.end || "-"}; ${observationLabel}`;

  const rows = [{
    scope: "all",
    label: `singles ${summary.single_count || 0} / coupons ${summary.coupon_count || 0}`,
    played_count: summary.placed_count || 0,
    settled_count: summary.settled_count || 0,
    open_count: summary.open_count || 0,
    awaiting_result_count: summary.awaiting_result_count || 0,
    truth_observation_count: summary.truth_observation_count || 0,
    turnover: summary.turnover || 0,
    open_exposure: summary.open_exposure || 0,
    awaiting_result_exposure: summary.awaiting_result_exposure || 0,
    profit_loss: summary.realized_profit_loss || 0,
    hit_rate: summary.hit_rate,
    average_odds: summary.average_odds
  }].concat((report.by_sport || []).map((item) => ({
    scope: item.sport_key || "unknown",
    label: `singles ${item.single_count || 0} / coupons ${item.coupon_count || 0}`,
    played_count: item.placed_count || 0,
    settled_count: item.settled_count || 0,
    open_count: item.open_count || 0,
    awaiting_result_count: item.awaiting_result_count || 0,
    truth_observation_count: item.truth_observation_count || 0,
    turnover: item.turnover || 0,
    open_exposure: item.open_exposure || 0,
    awaiting_result_exposure: item.awaiting_result_exposure || 0,
    profit_loss: item.realized_profit_loss || 0,
    hit_rate: item.hit_rate,
    average_odds: item.average_odds
  })));

  $(tableId).innerHTML = rows.map((item) => `
    <tr>
      <td><span class="pill">${esc(item.scope)}</span><br><span class="label">${esc(item.label)} / avg @ ${item.average_odds ? num(item.average_odds) : "-"}</span></td>
      <td>${esc(item.played_count)}<br><span class="muted">${money(item.turnover)}</span></td>
      <td>${esc(item.settled_count)}<br><span class="muted">truth ${esc(item.truth_observation_count || 0)}</span></td>
      <td>${esc(item.open_count)}<br><span class="muted">${money(item.open_exposure)}</span><br><span class="label">awaiting ${esc(item.awaiting_result_count || 0)} / ${money(item.awaiting_result_exposure)}</span></td>
      <td>${money(item.profit_loss)}</td>
      <td>${pct(item.hit_rate)}</td>
    </tr>
  `).join("");
  if (!Number(summary.placed_count || 0) && !(report.by_sport || []).length) {
    $(tableId).innerHTML = `<tr><td colspan="6" class="muted">No paper placements recorded for ${esc(label)}.</td></tr>`;
  }
}
function renderDailyPerformanceRecent(items) {
  $("daily-performance-recent").innerHTML = items.map((item) => {
    const selection = item.item_type === "coupon"
      ? `${item.market_name || "coupon"} / ${item.outcome_name || ""}`
      : `${item.event_name || item.item_id || ""} / ${item.outcome_name || ""}`;
    const context = [item.sport_key, item.competition, item.market_kind].filter(Boolean).join(" / ");
    const overdue = item.overdue_minutes === null || item.overdue_minutes === undefined ? null : Number(item.overdue_minutes);
    const overdueLabel = overdue === null ? "" : overdue > 0 ? `${Math.round(overdue)}m overdue` : "not due";
    const lookupLabel = item.last_lookup_at ? `last ${item.last_lookup_at}` : "no lookup yet";
    const lookupSource = [item.last_lookup_source_key, item.last_lookup_recommendation].filter(Boolean).join(" / ");
    const checkMeta = overdueLabel || lookupLabel;
    const lookupMeta = [overdueLabel ? lookupLabel : "", lookupSource].filter(Boolean).join(" / ");
    const observation = [item.latest_observation_result, item.latest_observation_source].filter(Boolean).join(" / ");
    const observationMeta = item.latest_observation_at
      ? `${observation}${item.latest_observation_confidence === null || item.latest_observation_confidence === undefined ? "" : ` / ${pct(item.latest_observation_confidence)}`} / ${item.latest_observation_at}`
      : observation;
    return `
      <tr>
        <td>${esc(item.created_at || "-")}</td>
        <td><span class="pill">${esc(item.item_type || "single")}</span></td>
        <td>${esc(selection)}<br><span class="label">${esc(context)}</span></td>
        <td>${money(item.stake)}<br><span class="muted">@ ${esc(item.observed_odds ?? "-")}</span></td>
        <td>${esc(item.expected_result_check_after || "-")}<br><span class="${overdue && overdue > 120 ? "danger" : "muted"}">${esc(checkMeta)}</span>${lookupMeta ? `<br><span class="label">${esc(lookupMeta)}</span>` : ""}</td>
        <td>${esc(item.status || "-")}<br><span class="muted">P/L ${maybeMoney(item.profit_loss)}</span>${observationMeta ? `<br><span class="label">truth ${esc(observationMeta)}</span>` : ""}</td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("daily-performance-recent").innerHTML = `<tr><td colspan="6" class="muted">No paper placements for the selected date.</td></tr>`;
  }
}
async function loadDailyPerformanceForDate(value) {
  const date = String(value || "").trim();
  if (!date) {
    $("daily-performance-window").textContent = "Select a date to load a daily report.";
    $("daily-performance").innerHTML = `<tr><td colspan="6" class="muted">No date selected.</td></tr>`;
    $("daily-performance-recent").innerHTML = `<tr><td colspan="6" class="muted">No date selected.</td></tr>`;
    return;
  }
  $("daily-performance-window").textContent = `Loading ${date}...`;
  const report = await json(api(`/api/performance/day?date=${encodeURIComponent(date)}`));
  renderSelectedDailyPerformance(report);
}
function renderPerformanceHistory(history) {
  const items = history.items || [];
  $("performance-history").innerHTML = items.map((item) => {
    const performance = item.performance || {};
    const ledger = item.ledger || {};
    const capacity = performance.placement_capacity || {};
    const settlement = performance.settlement_work || {};
    const blocked = capacity.blocked ? "blocked" : `${capacity.next_scan_capacity ?? 0}/${capacity.per_scan_limit ?? 0}`;
    return `
      <tr>
        <td>${esc(item.created_at || "-")}<br><span class="label">${esc(item.odds_snapshot_id || "")}</span></td>
        <td>${esc(item.source || "-")}</td>
        <td>${money(ledger.open_exposure)}</td>
        <td>${esc(settlement.due_total ?? 0)}</td>
        <td>${money(ledger.profit_loss)}</td>
        <td><span class="pill">${esc(blocked)}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("performance-history").innerHTML = `<tr><td colspan="6" class="muted">No persisted performance snapshots yet. Run a scan.</td></tr>`;
  }
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
function renderCouponRules(rules) {
  const items = rules.items || [];
  const summary = rules.summary || [];
  const totalRules = summary.reduce((sum, item) => sum + Number(item.rule_count || 0), 0);
  $("coupon-rules-count").textContent = String(totalRules);
  $("coupon-rules").innerHTML = items.map((item) => {
    const bounds = [
      item.minimum_accumulator !== null && item.minimum_accumulator !== undefined ? `min ${item.minimum_accumulator}` : null,
      item.maximum_accumulator !== null && item.maximum_accumulator !== undefined ? `max ${item.maximum_accumulator}` : null
    ].filter(Boolean).join(" / ");
    const label = [item.market_name, item.group_code].filter(Boolean).join(" / ");
    return `
      <tr>
        <td><span class="pill">${esc(item.sport_key)}</span><br><span class="label">${esc(item.competition_name || "")}</span></td>
        <td>${esc(label || item.market_id || "-")}<br><span class="muted">${esc(item.market_kind || "")}</span></td>
        <td>${esc(bounds || "not bounded")}</td>
        <td>${esc(item.restriction_scope || "-")}<br><span class="muted">unknown cross-market exclusions preserved in payload</span></td>
        <td>${esc(item.observed_at || "-")}<br><span class="label">${esc(item.snapshot_id || "")}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("coupon-rules").innerHTML = `<tr><td colspan="5" class="muted">No provider accumulator metadata has been observed yet. Run a scan.</td></tr>`;
  }
}
function renderOddsMovement(movement) {
  const items = movement.items || [];
  const summary = movement.summary || {};
  $("odds-moves-count").textContent = String(summary.returned_count || items.length);
  $("odds-movement").innerHTML = items.map((item) => {
    const moveClass = item.direction === "up" ? "ok" : item.direction === "down" ? "danger" : "muted";
    const delta = item.decimal_odds_delta === null || item.decimal_odds_delta === undefined
      ? "-"
      : `${Number(item.decimal_odds_delta) >= 0 ? "+" : ""}${Number(item.decimal_odds_delta).toFixed(2)}`;
    const pctMove = item.decimal_odds_delta_pct === null || item.decimal_odds_delta_pct === undefined
      ? "-"
      : `${Number(item.decimal_odds_delta_pct) >= 0 ? "+" : ""}${(Number(item.decimal_odds_delta_pct) * 100).toFixed(1)}%`;
    const status = [
      item.current_active === false ? "inactive" : null,
      item.current_displayed === false ? "hidden" : null
    ].filter(Boolean).join(", ");
    return `
      <tr>
        <td><span class="pill">${esc(item.sport_key)}</span> ${esc(item.event_name || item.event_id || "")}<br><span class="label">${esc(item.market_name || "")} / ${esc(item.outcome_name || "")}</span></td>
        <td>${num(item.previous_decimal_odds)}<br><span class="muted">${esc(item.previous_observed_at || "")}</span></td>
        <td>${num(item.current_decimal_odds)}<br><span class="muted">${esc(status || "active/displayed")}</span></td>
        <td><span class="${moveClass}">${esc(delta)}</span><br><span class="muted">${esc(pctMove)}</span></td>
        <td>${esc(item.current_observed_at || "-")}<br><span class="label">${esc(item.snapshot_id || "")}</span></td>
      </tr>
    `;
  }).join("");
  if (!items.length) {
    $("odds-movement").innerHTML = `<tr><td colspan="5" class="muted">No repeated outcome observations yet. Run at least two scans with overlapping events.</td></tr>`;
  }
}
function renderIntelligence(coverage) {
  const features = coverage.features || [];
  const totalFeatures = features.reduce((sum, item) => sum + Number(item.feature_count || 0), 0);
  $("feature-snapshots").textContent = String(totalFeatures);
  $("intelligence").innerHTML = features.map((item) => {
    const missing = [
      item.missing_weather_count ? `weather ${item.missing_weather_count}` : null,
      item.missing_news_count ? `news ${item.missing_news_count}` : null,
      item.missing_rankings_count ? `rankings ${item.missing_rankings_count}` : null,
      item.missing_form_count ? `form ${item.missing_form_count}` : null
    ].filter(Boolean).join(", ");
    return `
      <tr>
        <td><span class="pill">${esc(item.sport_key)}</span></td>
        <td>${esc(item.event_count)}</td>
        <td>${esc(item.feature_count)}</td>
        <td>${pct(item.average_confidence)}</td>
        <td>${esc(missing || "-")}</td>
      </tr>
    `;
  }).join("");
  if (!features.length) {
    $("intelligence").innerHTML = `<tr><td colspan="5" class="muted">No feature snapshots yet. Run a scan.</td></tr>`;
  }
  const runs = coverage.recent_runs || [];
  $("ingestion-runs").innerHTML = runs.map((item) => `
    <tr>
      <td>${esc(item.completed_at || "-")}<br><span class="label">${esc(item.snapshot_id || "")}</span></td>
      <td>${esc(item.source_key || "-")}</td>
      <td><span class="pill">${esc(item.status || "-")}</span></td>
      <td>${esc((item.sport_keys || []).join(", "))}</td>
      <td>${esc(item.event_count ?? 0)}</td>
    </tr>
  `).join("");
  if (!runs.length) {
    $("ingestion-runs").innerHTML = `<tr><td colspan="5" class="muted">No ingestion runs yet. Run a scan.</td></tr>`;
  }
}
function renderAuditEvents(events) {
  const items = events.items || [];
  $("audit-events").innerHTML = items.map((item) => `
    <tr>
      <td>${esc(item.created_at || "-")}<br><span class="label">${esc(item.id || "")}</span></td>
      <td><span class="pill">${esc(item.event_type || "-")}</span></td>
      <td><pre>${esc(JSON.stringify(item.details || {}, null, 2))}</pre></td>
    </tr>
  `).join("");
  if (!items.length) {
    $("audit-events").innerHTML = `<tr><td colspan="3" class="muted">No audit events yet.</td></tr>`;
  }
}
function renderPromotionGates(gates) {
  $("hermes-promotion-gates").innerHTML = (gates || []).map((gate) => {
    const policy = gate.policy || {};
    const blockers = gate.blockers || [];
    return `
      <tr>
        <td>${esc(gate.title || gate.experiment_id || "-")}<br><span class="label">${esc(gate.variable_name || "")}</span></td>
        <td><span class="pill ${gate.eligible_for_promotion ? "ok" : "danger"}">${gate.eligible_for_promotion ? "yes" : "no"}</span></td>
        <td>
          settled ${esc(policy.settled_paper_positions ?? 0)} / ${esc(policy.min_settled_paper_positions ?? "-")}<br>
          <span class="label">open ${esc(policy.open_or_awaiting_paper_positions ?? 0)}, replay ${policy.replay_evidence_present ? "yes" : "no"}</span>
        </td>
        <td>${blockers.length ? blockers.map(esc).join("<br>") : "<span class=\"ok\">clear</span>"}</td>
        <td>${esc(gate.recommendation || "")}</td>
      </tr>
    `;
  }).join("");
  if (!(gates || []).length) {
    $("hermes-promotion-gates").innerHTML = `<tr><td colspan="5" class="muted">No active experiments require promotion gating.</td></tr>`;
  }
}
function renderHermesCycle(hermes) {
  const cycle = hermes.latest_cycle || null;
  if (!cycle || !cycle.details) {
    $("hermes-cycle").innerHTML = `<tr><td colspan="5" class="muted">No Hermes cycle has completed yet.</td></tr>`;
    return;
  }
  const details = cycle.details || {};
  const replay = details.replay_refresh || {};
  const strategy = details.strategy || {};
  const reflection = details.reflection || {};
  const safety = details.safety || {};
  const refreshed = replay.refreshed_count ?? 0;
  const skipped = replay.skipped_count ?? 0;
  const replayState = replay.error
    ? `<span class="danger">${esc(replay.error)}</span>`
    : `${esc(refreshed)} refreshed<br><span class="label">${esc(skipped)} skipped</span>`;
  const safetyText = [
    safety.browser_control === false ? "no browser" : "browser?",
    safety.credential_access === false ? "no credentials" : "credentials?",
    safety.real_money_placement === false ? "no real money" : "real money?"
  ].join(", ");
  $("hermes-cycle").innerHTML = `
    <tr>
      <td>${esc(cycle.created_at || "-")}<br><span class="label">${esc(cycle.id || "")}</span></td>
      <td>${esc(details.trigger || "-")}<br><span class="label">${details.paper_only ? "paper-only" : "check safety"}</span></td>
      <td>${esc(reflection.id || "-")}<br><span class="label">${esc(reflection.status || "")}</span></td>
      <td>${replayState}<br><span class="label">experiments ${esc(strategy.experiment_count ?? 0)}, proposed ${esc(strategy.proposed_experiment_count ?? 0)}</span></td>
      <td>${esc(safetyText)}</td>
    </tr>
  `;
}
function renderStrategy(strategy, gates) {
  const experiments = strategy.experiments || [];
  const gateByExperiment = new Map((gates || []).map((gate) => [gate.experiment_id, gate]));
  $("experiments").innerHTML = experiments.map((item) => {
    const evidence = item.evidence || {};
    const gate = gateByExperiment.get(item.id);
    const change = `${JSON.stringify(item.baseline_value)} -> ${JSON.stringify(item.proposed_value)}`;
    const evidenceParts = [];
    if (evidence.long_price_candidate_count !== undefined) {
      evidenceParts.push(`${evidence.long_price_candidate_count} long-price`);
    }
    if (evidence.specialized_market_candidate_count !== undefined) {
      evidenceParts.push(`${evidence.specialized_market_candidate_count} specialized`);
    }
    if (evidence.provider_supported_double_candidate_count !== undefined) {
      evidenceParts.push(`${evidence.provider_supported_double_candidate_count} double-ready`);
    }
    if (evidence.large_odds_movement_candidate_count !== undefined) {
      evidenceParts.push(`${evidence.large_odds_movement_candidate_count} large moves`);
    }
    const replay = item.decision_payload && item.decision_payload.replay_evidence
      ? item.decision_payload.replay_evidence
      : null;
    if (replay && replay.delta) {
      evidenceParts.push(`replay ${replay.delta.selected_count >= 0 ? "+" : ""}${replay.delta.selected_count} selected`);
    }
    const evidenceText = evidenceParts.length ? evidenceParts.join(", ") : `${evidence.candidate_count ?? "-"} candidates`;
    const canApprove = item.status === "proposed";
    const canReplay = ["proposed", "approved_for_replay", "active_simulation"].includes(item.status);
    const canActivate = item.status === "approved_for_replay" && !!replay;
    const canPromote = item.status === "active_simulation" && !!replay && !!gate && !!gate.eligible_for_promotion;
    return `
      <tr>
        <td>${esc(item.status)}</td>
        <td>${esc(item.variable_name)}<br><span class="label">${esc(item.title)}</span></td>
        <td>${esc(change)}</td>
        <td>${esc(evidenceText)}<br><span class="label">${esc(evidence.snapshot_id || "")}</span></td>
        <td>
          <div class="actions">
            <button data-exp="${item.id}" data-action="approve" ${!canApprove ? "disabled" : ""}>Approve</button>
            <button data-exp="${item.id}" data-action="reject" ${!canApprove ? "disabled" : ""}>Reject</button>
            <button data-exp="${item.id}" data-action="replay" ${!canReplay ? "disabled" : ""}>Replay</button>
            <button data-exp="${item.id}" data-action="activate" ${!canActivate ? "disabled" : ""}>Activate</button>
            <button data-exp="${item.id}" data-action="promote" title="${esc(gate && gate.recommendation ? gate.recommendation : "Promotion requires Hermes gate clearance.")}" ${!canPromote ? "disabled" : ""}>Promote</button>
          </div>
        </td>
      </tr>
    `;
  }).join("");
  if (!experiments.length) {
    $("experiments").innerHTML = `<tr><td colspan="5" class="muted">No experiment proposals yet. Run a scan.</td></tr>`;
  }
  document.querySelectorAll("[data-exp]").forEach((button) => {
    button.addEventListener("click", async () => {
      await json(api("/api/strategy/experiment/review"), {
        method: "POST",
        body: JSON.stringify({
          experiment_id: button.dataset.exp,
          action: button.dataset.action,
          notes: "operator web-ui action"
        })
      });
      await load();
    });
  });
}
async function load() {
  const status = await json(api("/api/status"));
  $("mode").textContent = status.mode;
  $("database").textContent = status.database.connected ? "connected" : "degraded";
  $("database").className = status.database.connected ? "value ok" : "value danger";
  const scanner = status.scanner || {};
  $("snapshot").textContent = status.latest_snapshot_id
    ? `${status.latest_snapshot_id} (${mins(scanner.latest_snapshot_age_seconds)} old)`
    : "-";
  $("scan-cadence").textContent = `${mins(scanner.interval_seconds)} / ${scanner.scan_limit ?? "-"} sports`;
  $("next-scan-due").textContent = scanner.next_scan_due_at || "now";
  $("next-scan-due").className = scanner.due ? "value danger" : "value ok";
  $("placement").textContent = status.allow_real_money_placement ? "enabled" : "disabled";
  const autoPaper = status.auto_paper || {};
  $("auto-paper-state").textContent = autoPaper.enabled
    ? `${autoPaper.per_scan_limit || 0} x ${money(autoPaper.default_stake || 0)}`
    : "off";
  const hermesStatus = status.hermes || {};
  $("hermes-loop").textContent = hermesStatus.enabled
    ? `${mins(hermesStatus.reflection_interval_seconds || 0)}`
    : "off";
  $("hermes-loop").className = hermesStatus.enabled ? "value ok" : "value muted";
  const summary = await json(api("/api/ledger/summary"));
  $("awaiting-result").textContent = String((summary.by_status || {}).awaiting_result || 0);
  $("exposure").textContent = money(summary.open_exposure);
  $("profit").textContent = money(summary.profit_loss);
  $("profit").className = Number(summary.profit_loss || 0) >= 0 ? "value ok" : "value danger";
  const candidates = await json(api("/api/candidates"));
  renderRows(candidates.items || []);
  const decisions = await json(api("/api/strategy/decisions"));
  renderStrategyDecisions(decisions.items || []);
  const coupons = await json(api("/api/coupons"));
  renderCoupons(coupons.items || []);
  const settlementSources = await json(api("/api/settlement/sources"));
  renderSettlementSources(settlementSources);
  const externalResultLinks = await json(api("/api/settlement/source-links"));
  renderExternalResultLinksTable(externalResultLinks);
  const entityAliases = await json(api("/api/aliases"));
  renderEntityAliases(entityAliases);
  const simulatedCoupons = await json(api("/api/coupons/simulated"));
  renderSimulatedCoupons(simulatedCoupons.items || []);
  const ledger = await json(api("/api/ledger"));
  renderLedger(ledger.items || []);
  const settlementReview = await json(api("/api/settlement/review"));
  renderSettlementReview(settlementReview);
  const resultAgentQueue = await json(api("/api/result-agent/queue"));
  renderResultAgentQueue(resultAgentQueue);
  const accountHistoryRequests = await json(api("/api/result-agent/account-requests"));
  renderAccountHistoryRequests(accountHistoryRequests);
  const settlementObservations = await json(api("/api/settlement/observations"));
  renderSettlementObservations(settlementObservations);
  const externalResultEvidence = await json(api("/api/settlement/external-evidence"));
  renderExternalResultEvidence(externalResultEvidence);
  const settlementLookupAttempts = await json(api("/api/settlement/lookup-attempts"));
  renderSettlementLookupAttempts(settlementLookupAttempts);
  const played = await json(api("/api/strategy/played"));
  renderPlayed(played);
  const performance = await json(api("/api/performance"));
  renderPerformance(performance);
  const todayPerformance = await json(api("/api/performance/today"));
  renderTodayPerformance(todayPerformance);
  const yesterdayPerformance = await json(api("/api/performance/yesterday"));
  renderYesterdayPerformance(yesterdayPerformance);
  if (!$("daily-performance-date").value && yesterdayPerformance.local_date) {
    $("daily-performance-date").value = yesterdayPerformance.local_date;
    renderSelectedDailyPerformance(yesterdayPerformance);
  }
  const performanceHistory = await json(api("/api/performance/history"));
  renderPerformanceHistory(performanceHistory);
  const coverage = await json(api("/api/catalog/coverage"));
  renderCoverage(coverage);
  const couponRules = await json(api("/api/coupon-rules"));
  renderCouponRules(couponRules);
  const oddsMovement = await json(api("/api/odds/movement"));
  renderOddsMovement(oddsMovement);
  const intelligence = await json(api("/api/intelligence/coverage"));
  renderIntelligence(intelligence);
  const auditEvents = await json(api("/api/audit/events"));
  renderAuditEvents(auditEvents);
  const hermes = await json(api("/api/hermes"));
  renderStrategy(hermes.strategy || {}, hermes.promotion_gates || []);
  renderHermesCycle(hermes);
  renderPromotionGates(hermes.promotion_gates || []);
  $("hermes").textContent = JSON.stringify(hermes, null, 2);
}
$("scan").addEventListener("click", async () => {
  $("scan").disabled = true;
  try { await json(api("/api/scan"), { method: "POST", body: "{}" }); await load(); }
  finally { $("scan").disabled = false; }
});
$("auto-paper").addEventListener("click", async () => {
  $("auto-paper").disabled = true;
  try { await json(api("/api/simulate/selected"), { method: "POST", body: "{}" }); await load(); }
  finally { $("auto-paper").disabled = false; }
});
$("generate-coupons").addEventListener("click", async () => {
  $("generate-coupons").disabled = true;
  try { await json(api("/api/coupons/generate"), { method: "POST", body: "{}" }); await load(); }
  finally { $("generate-coupons").disabled = false; }
});
$("auto-paper-coupons").addEventListener("click", async () => {
  $("auto-paper-coupons").disabled = true;
  try { await json(api("/api/coupons/simulate/selected"), { method: "POST", body: "{}" }); await load(); }
  finally { $("auto-paper-coupons").disabled = false; }
});
$("queue-settlement").addEventListener("click", async () => {
  $("queue-settlement").disabled = true;
  try { await json(api("/api/ledger/queue"), { method: "POST", body: "{}" }); await load(); }
  finally { $("queue-settlement").disabled = false; }
});
$("review-settlement").addEventListener("click", async () => {
  $("review-settlement").disabled = true;
  try { await json(api("/api/settlement/review"), { method: "POST", body: "{}" }); await load(); }
  finally { $("review-settlement").disabled = false; }
});
$("run-result-agent").addEventListener("click", async () => {
  $("run-result-agent").disabled = true;
  try { await json(api("/api/result-agent/run"), { method: "POST", body: "{}" }); await load(); }
  finally { $("run-result-agent").disabled = false; }
});
$("load-daily-performance").addEventListener("click", async () => {
  $("load-daily-performance").disabled = true;
  try { await loadDailyPerformanceForDate($("daily-performance-date").value); }
  catch (error) {
    $("daily-performance-window").textContent = `Daily report failed: ${error.message || error}`;
    $("daily-performance").innerHTML = `<tr><td colspan="6" class="muted">Could not load daily report.</td></tr>`;
    $("daily-performance-recent").innerHTML = `<tr><td colspan="6" class="muted">Could not load daily placements.</td></tr>`;
  }
  finally { $("load-daily-performance").disabled = false; }
});
$("daily-performance-date").addEventListener("change", async () => {
  try { await loadDailyPerformanceForDate($("daily-performance-date").value); }
  catch (error) {
    $("daily-performance-window").textContent = `Daily report failed: ${error.message || error}`;
    $("daily-performance-recent").innerHTML = `<tr><td colspan="6" class="muted">Could not load daily placements.</td></tr>`;
  }
});
$("commit-settlements").addEventListener("click", async () => {
  const pending = Array.from(pendingSettlementReviews.values());
  if (!pending.length) return;
  $("commit-settlements").disabled = true;
  try {
    for (const item of pending) {
      if (item.type === "coupon") {
        await json(api("/api/coupons/settle"), {
          method: "POST",
          body: JSON.stringify({
            coupon_id: item.id,
            result: item.result,
            source: item.source,
            confidence: 1,
            notes: "operator batch settlement review"
          })
        });
      } else {
        await json(api("/api/ledger/settle"), {
          method: "POST",
          body: JSON.stringify({
            bet_id: item.id,
            result: item.result,
            source: item.source,
            confidence: 1,
            notes: "operator batch settlement review"
          })
        });
      }
    }
    pendingSettlementReviews.clear();
    await load();
  } finally {
    updatePendingSettlementUi();
  }
});
$("reflect-yesterday").addEventListener("click", async () => {
  $("reflect-yesterday").disabled = true;
  try { await json(api("/api/hermes/reflect/yesterday"), { method: "POST", body: "{}" }); await load(); }
  finally { $("reflect-yesterday").disabled = false; }
});
$("run-hermes").addEventListener("click", async () => {
  $("run-hermes").disabled = true;
  try { await json(api("/api/hermes/run"), { method: "POST", body: "{}" }); await load(); }
  finally { $("run-hermes").disabled = false; }
});
$("refresh").addEventListener("click", load);
load().catch((error) => { $("reasoning").textContent = error.stack || String(error); });
"#;
