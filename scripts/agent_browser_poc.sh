#!/usr/bin/env bash
set -euo pipefail

SESSION_NAME="${AGENT_BROWSER_SESSION_NAME:-danske-spil-poc}"
ALLOWED_DOMAINS="danskespil.dk,*.danskespil.dk"
OUT_DIR="${1:-tmp/browser-observations}"

mkdir -p "$OUT_DIR"

agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" open "https://danskespil.dk/oddset"
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" wait 4000

# The first run may be blocked by the cookie consent modal. This selects the
# most restrictive visible option when it is present.
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" eval '(() => {
  const buttons = [...document.querySelectorAll("button")];
  const deny = buttons.find((button) => /Fravælg alle/i.test(button.innerText || button.textContent || ""));
  if (!deny || deny.offsetParent === null) return { clicked: false };
  deny.click();
  return { clicked: true };
})()'
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" wait 1000

agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" screenshot "$OUT_DIR/oddset-home.png"
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" snapshot -i > "$OUT_DIR/oddset-home.snapshot.txt"
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" eval '(() => {
  const text = (el) => (el.innerText || el.textContent || el.getAttribute("aria-label") || "").replace(/\s+/g, " ").trim();
  const links = [...document.querySelectorAll("a")].map((a) => ({ text: text(a), href: a.href, className: a.className })).filter((x) => x.text || x.href);
  const buttons = [...document.querySelectorAll("button")].map((b) => ({ text: text(b), disabled: b.disabled, className: b.className })).filter((x) => x.text);
  const marketChips = buttons.filter((b) => /Kampvinder|Antal|Handicap|Begge|Dobbelt|Spillelinjer|Set|Game|Quarter|Halvleg|Over|Under/i.test(b.text));
  const oddsButtons = buttons.filter((b) => /(^|\s)(1|X|2|O|U)\s?[+-]?\d*[\.,]?\d*\s+\d+[\.,]\d+|Tilføj\s+\d+[\.,]\d+/.test(b.text));
  const sportsLinks = links.filter((l) => /\/oddset\/sport\//.test(l.href));
  const eventLinks = links.filter((l) => /\/oddset\/sports\/event\//.test(l.href));
  return { url: location.href, title: document.title, sportsLinks, eventLinks: eventLinks.slice(0, 100), marketChips, oddsButtons: oddsButtons.slice(0, 200) };
})()' > "$OUT_DIR/oddset-home.dom.json"

agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" open "https://danskespil.dk/tips"
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" wait 4000
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" screenshot "$OUT_DIR/tips-home.png"
agent-browser --session-name "$SESSION_NAME" --allowed-domains "$ALLOWED_DOMAINS" snapshot -i > "$OUT_DIR/tips-home.snapshot.txt"
