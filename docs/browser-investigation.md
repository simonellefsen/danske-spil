# Browser Investigation Plan

The first browser phase is a structured reconnaissance pass over Oddset and Tips. It must not submit bets or mutate account state.

## Tooling

Preferred approach:

- Use `agent-browser` sessions or Playwright with a persistent browser profile.
- Use a normal browser engine. Do not fake device identity to bypass controls.
- Keep screenshots and traces sanitized before committing.
- Store credentials only in `.env.local`; never print them to logs.

Session names:

- `danske-spil-oddset-investigation`
- `danske-spil-tips-investigation`

## Investigation Scope

Oddset:

- Landing page and product navigation.
- Login entry point and post-login state.
- Sport, league, event, market, odds, betslip, stake, and confirmation states.
- How odds changes are surfaced.
- How disabled/suspended selections are represented.
- How account limits or warnings appear.

Tips:

- Coupon discovery and navigation.
- Row/match model.
- Selection controls.
- Coupon validation and stake/price behavior.
- Confirmation boundary.

Shared:

- Cookie banners and consent state.
- Responsive/mobile vs desktop layout differences.
- Network calls that expose public-ish event/odds data.
- Selector stability and test IDs.
- Error, maintenance, timeout, login expiry, and responsible-gambling states.

## Stop Boundaries

The investigator must stop before:

- Final coupon/bet submission.
- Deposit or withdrawal flow.
- Payment card flow.
- Bonus activation.
- Account setting changes.
- Limit increases.
- MitID or Spil-ID manual verification beyond user-approved login.
- CAPTCHA or bot-detection bypass attempts.

## Capture Template

For each page or state:

```yaml
observed_at:
product: oddset | tips
url:
auth_state: anonymous | logged_in | expired | blocked
viewport:
stable_selectors:
  - selector:
    meaning:
    confidence: low | medium | high
state_transitions:
  - action:
    before:
    after:
network_observations:
  - method:
    url_pattern:
    payload_notes:
risks:
  - note:
screenshots:
  - path:
```

## Deliverables

- `wiki/sources/danske-spil-dom-observations.md`
- `wiki/concepts/oddset-navigation-model.md`
- `wiki/concepts/tips-navigation-model.md`
- A first read-only selector map for implementation.
- A list of unknowns and blocked states requiring human review.
