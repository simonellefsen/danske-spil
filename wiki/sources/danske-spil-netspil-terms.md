---
type: source-note
tags:
  - danske-spil/wiki
  - compliance
  - source
updated: 2026-05-25
sources:
  - https://danskespil.dk/regler_og_vilkaar/vilkaar/vilkaar-dli
  - https://danskespil.dk/om/spil-med-omtanke/kontakt_spilmedomtanke
  - https://danskespil.dk/spilid
---

# Danske Spil Net-Game Terms And Responsible-Gambling Sources

This note summarizes official pages reviewed during initial planning on 2026-05-25. Re-check the official pages before implementation that touches account, login, or betting behavior.

## Sources Reviewed

- Danske Licens Spil net-game terms for the Blue Account.
- Danske Spil responsible-gambling contact page.
- Danske Spil Spil-ID page and responsible-gambling links.

## Relevant Terms Summary

The net-game terms say online participation requires a registered account, and registration requires a Danish/Greenlandic or Faroese CPR number, age 18+, no personal bankruptcy, and legal capacity.

The terms give Danske Licens Spil broad rights to block or restrict an account, limit deposits, cancel games, and reverse funds when it detects or suspects listed conduct.

Relevant listed conduct includes:

- Cheating, fraud, illegal activity, match fixing, and odds manipulation.
- Syndicate-like coordinated play.
- Systematic play on delayed market movement.
- Systematic play on erroneous odds or exploitation of bet delays.
- Use of applications, robots, or similar mechanisms to affect or automate play.
- Misuse of another player's account, contact details, personal data, or login details.

## Project Implications

- Treat direct automation of play as a terms risk.
- Do not build unattended real-money bet placement in the initial system.
- Use browser automation only for non-mutating investigation unless a later decision record explicitly accepts the risk.
- Keep Hermes away from browser control, credentials, cookies, and final submission actions.
- Add local responsible-gambling limits that are stricter than site controls.

## Responsible-Gambling Notes

Danske Spil publishes responsible-gambling guidance, contact information, and links to StopSpillet. The project should include cooldowns, stake limits, no loss-chasing, and audit logs before any betting-critical feature is considered.

## Open Follow-Up

- Confirm whether Danske Spil has an official developer, odds feed, affiliate, or data export route that is permissible for this use case.
- Re-check terms immediately before implementing anything beyond observation and simulation.
