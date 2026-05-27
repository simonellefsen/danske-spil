# danske-spil

Research-first automation workspace for Danske Spil Oddset and Tips.

The initial project goal is to understand the site, maintain a durable LLM wiki, design a guarded multi-agent architecture, and prepare a Docker Desktop Kubernetes deployment shape. Real-money bet placement is deliberately out of scope for the first implementation pass.

## Current Status

- Active POC implementation is Rust with a Dioxus-rendered web UI shell.
- Runtime deployments use a multi-stage Docker build with a `scratch` final image.
- `.env.local` is ignored and must hold credentials locally.
- The `gambler` agent may observe, model, and prepare candidate coupons.
- `gambler` should scan and monitor markets, simulate bet placement, keep a ledger, and reconcile final outcomes.
- The worker should run about every 15 minutes to find new paper opportunities and check queued bets for verified outcomes.
- Strategies should be able to model singles and provider-supported multi-leg coupons such as doubles, triples, and larger accumulators, subject to sport/category constraints.
- `gambler` should enrich decisions with sport stats, trends, weather, seasonality, and news stored in Postgres.
- The proposed `gambler` web UI should show candidate odds, structured reasoning, risk checks, and review state.
- The proposed Hermes Agent loop may reflect and propose one-variable strategy experiments.
- No agent may submit real-money bets until a separate compliance and human-approval gate is explicitly accepted.

## Start Here

These topics are the durable map for the project:

- [Project plan](docs/project-plan.md) - Phased roadmap from research and browser investigation through simulation, Hermes experiments, and any future human-approved action surface.
- [Compliance and safety](docs/compliance-and-safety.md) - Hard guardrails for regulated gambling, credentials, responsible-gambling controls, and the current no-real-money automation posture.
- [Browser investigation](docs/browser-investigation.md) - Runbook for using `agent-browser` or an equivalent browser engine to learn Oddset and Tips without mutating account state.
- [Gambler web UI](docs/web-ui.md) - Operator dashboard requirements for observed odds, candidate reasoning, coupon review, Hermes state, audit events, and safety gates.
- [Sports data intelligence](docs/data-intelligence.md) - Data-ingestion plan for football/soccer, tennis, basketball, Formula 1, golf, and cycling using stats, trends, weather, seasonality, and news.
- [Simulation ledger](docs/simulation-ledger.md) - Paper-betting model for simulated placements, immutable entry odds, settlement lookup, and simulated performance metrics.
- [POC implementation notes](docs/poc-implementation.md) - Current Rust/Dioxus service shape, scanner behavior, storage model, API endpoints, and implementation boundaries.
- [POC deployment](docs/poc-deployment.md) - Local Docker Desktop Kubernetes deployment steps for the gambler API, worker, Hermes POC, and CloudNativePG database.
- [ngrok path routing](docs/ngrok-path-routing.md) - `/danske-spil` route expectations and shared gateway ownership.
- [Hermes and gambler loop](docs/hermes-agent.md) - Safe reinforcement loop design where Hermes can reflect and propose one-variable experiments without browser, secret, or bet-placement access.
- [Kubernetes architecture](docs/kubernetes-architecture.md) - Namespace layout, workloads, secrets, database cluster, observability expectations, and operational commands.
- [Project wiki](wiki/index.md) - Maintained knowledge base with concepts, runbooks, decisions, source notes, and experiment records.

## Operating Rule

Use `rtk` for shell commands in this repository.

```bash
rtk <command>
```

## Rust Checks

```bash
rtk cargo fmt
rtk cargo check
rtk cargo test
```

## Local Secrets

Create `.env.local` from [.env.example](.env.example). Keep all `DANSKESPIL_*`, Hermes keys, database passwords, cookies, and browser session material out of Git.

## First Milestone

Milestone 0 is a non-mutating investigation:

1. Open Oddset and Tips with a normal browser session.
2. Document navigation, login checkpoints, DOM selectors, and state transitions.
3. Capture sanitized screenshots and selector notes.
4. Build read-only odds and coupon candidate extraction.
5. Simulate candidate bet placement into a ledger.
6. Look up final outcomes and reconcile simulated win/loss.
7. Stop before any final bet confirmation or payment-like action.
