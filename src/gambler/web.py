from __future__ import annotations


INDEX_HTML = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Gambler POC</title>
  <style>
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
    .warn { color: #a16207; }
    pre { white-space: pre-wrap; overflow: auto; max-height: 420px; background: #0f172a; color: #e2e8f0; padding: 12px; border-radius: 6px; }
  </style>
</head>
<body>
  <header>
    <h1>Gambler POC</h1>
    <div class="toolbar">
      <button class="primary" id="scan">Scan markets</button>
      <button id="refresh">Refresh</button>
    </div>
  </header>
  <main>
    <div class="metric-row">
      <div class="metric"><div class="label">Mode</div><div class="value" id="mode">-</div></div>
      <div class="metric"><div class="label">Database</div><div class="value" id="database">-</div></div>
      <div class="metric"><div class="label">Latest snapshot</div><div class="value" id="snapshot">-</div></div>
      <div class="metric"><div class="label">Open exposure</div><div class="value" id="exposure">-</div></div>
      <div class="metric"><div class="label">Paper P/L</div><div class="value" id="profit">-</div></div>
      <div class="metric"><div class="label">Real-money placement</div><div class="value danger" id="placement">disabled</div></div>
    </div>
    <div class="grid">
      <section>
        <h2>Candidate odds</h2>
        <table>
          <thead><tr><th>Sport</th><th>Event</th><th>Market</th><th>Outcome</th><th>Odds</th><th>Score</th><th></th></tr></thead>
          <tbody id="candidates"></tbody>
        </table>
      </section>
      <section>
        <h2>Reasoning</h2>
        <pre id="reasoning">No scan loaded.</pre>
      </section>
      <section>
        <h2>Paper ledger</h2>
        <table>
          <thead><tr><th>Created</th><th>Selection</th><th>Stake</th><th>Status</th><th>P/L</th><th></th></tr></thead>
          <tbody id="ledger"></tbody>
        </table>
      </section>
      <section>
        <h2>Hermes view</h2>
        <pre id="hermes">No reflections yet.</pre>
      </section>
    </div>
  </main>
  <script>
    const $ = (id) => document.getElementById(id);
    const appBase = document.body.dataset.basePath || "";
    const api = (path) => `${appBase}${path}`;
    const money = (value) => Number(value || 0).toFixed(2);
    const pct = (value) => value === null || value === undefined ? "-" : `${(Number(value) * 100).toFixed(1)}%`;
    const num = (value) => value === null || value === undefined ? "-" : Number(value).toFixed(3);
    const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
    }[ch]));
    const json = (url, options) => fetch(url, options).then((r) => {
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
      const hermes = await json(api("/api/hermes"));
      $("hermes").textContent = JSON.stringify(hermes, null, 2);
    }
    $("scan").addEventListener("click", async () => {
      $("scan").disabled = true;
      try { await json(api("/api/scan"), { method: "POST" }); await load(); }
      finally { $("scan").disabled = false; }
    });
    $("refresh").addEventListener("click", load);
    load().catch((error) => { $("reasoning").textContent = error.stack || String(error); });
  </script>
</body>
</html>
"""
