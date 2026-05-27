# Compliance And Safety

This project touches regulated gambling, credentials, personal account state, and real money. The default posture is research-first, read-only, and human-approved.

## Official Sources Checked

Current official pages reviewed on 2026-05-25:

- Danske Licens Spil net-game terms: https://danskespil.dk/regler_og_vilkaar/vilkaar/vilkaar-dli
- Danske Spil responsible gambling contact page: https://danskespil.dk/om/spil-med-omtanke/kontakt_spilmedomtanke
- Spil-ID / responsible gambling page: https://danskespil.dk/spilid

## Hard Guardrails

- Do not bypass MitID, Spil-ID, age checks, geo checks, account controls, rate limits, CAPTCHA, bot checks, or responsible-gambling controls.
- Do not use another person's account, credentials, Spil-ID, payment instrument, or personal data.
- Do not automate deposits, withdrawals, bonus activation, account profile changes, self-exclusion settings, or limit increases.
- Do not let Hermes access credentials, cookies, browser profiles, raw account payloads, or Kubernetes secrets.
- Do not place unattended real-money bets.
- Do not exploit delayed odds, broken odds, bet delays, site defects, insider information, match fixing, syndicate play, or promotion loopholes.
- Stop immediately if the site displays an automation warning, account-risk warning, responsible-gambling intervention, CAPTCHA, MitID challenge, or explicit manual-only flow.

## Terms Risk

The current Blue Account terms list use of applications, robots, or similar mechanisms to affect or automate play as a possible basis for account restrictions and game/funds reversals. They also list systematic betting on delayed market movements, erroneous odds, and bet delay exploitation as problematic conduct.

Project implication:

- Browser automation can be used only for non-mutating investigation unless the operator knowingly accepts the account and terms risk later.
- Strategy learning must be based on snapshots, simulation, and human-reviewed evidence.
- Any final bet submission must remain outside the autonomous loop by default.

## Responsible-Gambling Controls

The system should enforce stricter local limits than the site requires:

- `DANSKESPIL_ALLOW_REAL_MONEY_PLACEMENT=false` by default.
- Single, daily, and weekly stake limits default to `0`.
- Cooldown after losses, streaks, or rapid bet cadence.
- No martingale, chase-loss, or "recover losses" staking modes.
- Audit every recommendation, rejection, approval, and placed bet.
- Show net exposure before any human confirmation.
- Keep StopSpillet and Danske Spil responsible-gambling links visible in operator docs.

## Required Approval Gate Before Real-Money Work

Before implementation may place or submit real-money bets, create a decision record that answers:

- What exactly do Danske Spil's current terms allow or prohibit?
- Is the account owner willing to accept account closure or reversal risk?
- What stake limits are acceptable?
- What manual confirmation step is required?
- How will audit logs prove the system did not bypass controls?
- The POC exposes recent `audit_events` through the web UI so operators can review scan, paper-placement, settlement, and strategy-review actions without database access.
- How will the system stop if gambling behavior looks unhealthy?

Until that decision exists, all implementation should be observe-only or simulation-only.
