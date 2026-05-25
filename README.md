# danske-spil

Research-first automation workspace for Danske Spil Oddset and Tips.

The initial project goal is to understand the site, maintain a durable LLM wiki, design a guarded multi-agent architecture, and prepare a Docker Desktop Kubernetes deployment shape. Real-money bet placement is deliberately out of scope for the first implementation pass.

## Current Status

- Documentation and wiki scaffold only.
- `.env.local` is ignored and must hold credentials locally.
- The proposed `gambler` agent may observe, model, and prepare candidate coupons.
- The proposed Hermes Agent loop may reflect and propose one-variable strategy experiments.
- No agent may submit real-money bets until a separate compliance and human-approval gate is explicitly accepted.

## Start Here

- [Project plan](/Users/lindau/codex/danske-spil/docs/project-plan.md)
- [Compliance and safety](/Users/lindau/codex/danske-spil/docs/compliance-and-safety.md)
- [Browser investigation](/Users/lindau/codex/danske-spil/docs/browser-investigation.md)
- [Hermes and gambler loop](/Users/lindau/codex/danske-spil/docs/hermes-agent.md)
- [Kubernetes architecture](/Users/lindau/codex/danske-spil/docs/kubernetes-architecture.md)
- [Project wiki](/Users/lindau/codex/danske-spil/wiki/index.md)

## Operating Rule

Use `rtk` for shell commands in this repository.

```bash
rtk <command>
```

## Local Secrets

Create `.env.local` from [.env.example](/Users/lindau/codex/danske-spil/.env.example). Keep all `DANSKESPIL_*`, Hermes keys, database passwords, cookies, and browser session material out of Git.

## First Milestone

Milestone 0 is a non-mutating investigation:

1. Open Oddset and Tips with a normal browser session.
2. Document navigation, login checkpoints, DOM selectors, and state transitions.
3. Capture sanitized screenshots and selector notes.
4. Build read-only odds and coupon candidate extraction.
5. Stop before any final bet confirmation or payment-like action.
