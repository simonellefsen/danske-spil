---
type: runbook
tags:
  - danske-spil/wiki
  - browser
  - investigation
updated: 2026-05-25
sources:
  - /Users/lindau/codex/danske-spil/docs/browser-investigation.md
---

# Browser Investigation Runbook

Use this checklist for the first Oddset/Tips investigation pass.

## Preflight

```bash
rtk ls -la .env.local
rtk rg -n "DANSKESPIL_" .env.example docs wiki
```

Confirm:

- `.env.local` exists and is ignored.
- Real-money placement flags remain false or zero.
- The session name is dedicated to investigation.
- No screenshots/traces will be committed before review.

## Oddset Pass

- Open `https://danskespil.dk/oddset`.
- Record cookie and consent state.
- Record anonymous navigation selectors.
- Log in only if needed and user-approved.
- Record sport, league, event, market, odds, betslip, and stake states.
- Stop before final submit.

## Tips Pass

- Open `https://danskespil.dk/tips`.
- Record coupon discovery and row model.
- Record selection controls and validation states.
- Stop before final submit.

## Write-Up

Create or update:

- `wiki/sources/danske-spil-dom-observations.md`
- `wiki/concepts/oddset-navigation-model.md`
- `wiki/concepts/tips-navigation-model.md`
- `wiki/log.md`

Do not include credentials, cookies, account identifiers, payment data, or personal details.
