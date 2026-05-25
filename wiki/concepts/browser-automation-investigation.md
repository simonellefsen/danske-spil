---
type: concept
tags:
  - danske-spil/wiki
  - browser
  - investigation
updated: 2026-05-25
sources:
  - ../../docs/browser-investigation.md
  - ../sources/danske-spil-netspil-terms.md
---

# Browser Automation Investigation

Browser automation is allowed in this project only as a non-mutating research tool until a later decision record changes the posture.

## Purpose

The investigation should learn:

- Page and navigation states.
- Stable selectors.
- Betslip/coupon state models.
- Login expiry behavior.
- Error and warning states.
- Public or semi-public network data shapes.

## Non-Mutating Boundary

Stop before final submission, deposits, withdrawals, bonus activation, account settings, limit increases, CAPTCHA bypass, or any site warning that asks for manual action.

## Artifact Rules

- Sanitize screenshots.
- Do not commit cookies, profiles, traces containing credentials, account identifiers, payment data, or personal data.
- File durable findings under `wiki/sources/` and `wiki/concepts/`.

## Related

- [browser investigation runbook](../runbooks/browser-investigation.md)
- [terms source note](../sources/danske-spil-netspil-terms.md)
