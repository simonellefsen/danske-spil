#!/usr/bin/env bash
set -euo pipefail

CONTEXT="${KUBE_CONTEXT:-docker-desktop}"
POLICY_NAMESPACE="${POLICY_NAMESPACE:-saxo-rust}"
POLICY_NAME="${POLICY_NAME:-daytrader-oauth}"
PUBLIC_ENDPOINT_NAME="${PUBLIC_ENDPOINT_NAME:-daytrader-frontend}"
GAMBLER_UPSTREAM_URL="${GAMBLER_UPSTREAM_URL:-http://gambler-api.danske-spil:8080}"
SAXO_INTERNAL_URL="${SAXO_INTERNAL_URL:-http://saxo-daytrader.internal:80}"

kubectl patch agentendpoints.ngrok.k8s.ngrok.com "$PUBLIC_ENDPOINT_NAME" \
  -n "$POLICY_NAMESPACE" \
  --context "$CONTEXT" \
  --type=merge \
  -p "{\"spec\":{\"upstream\":{\"protocol\":\"http1\",\"url\":\"$GAMBLER_UPSTREAM_URL\"}}}"

policy_json="$(kubectl get ngroktrafficpolicies.ngrok.k8s.ngrok.com "$POLICY_NAME" \
  -n "$POLICY_NAMESPACE" \
  --context "$CONTEXT" \
  -o json)"

patched_policy="$(POLICY_JSON="$policy_json" SAXO_INTERNAL_URL="$SAXO_INTERNAL_URL" python3 - <<'PY'
import json
import os

policy = json.loads(os.environ["POLICY_JSON"])
rules = policy["spec"]["policy"].get("on_http_request", [])

def expressions_text(rule):
    return "\n".join(rule.get("expressions") or [])

rules = [
    rule for rule in rules
    if "/danske-spil" not in expressions_text(rule)
    and "/saxo-daytrader" not in expressions_text(rule)
]

rules.append({
    "expressions": ['req.url.path.startsWith("/saxo-daytrader")'],
    "actions": [
        {
            "type": "url-rewrite",
            "config": {
                "from": "/saxo-daytrader/?(.*)",
                "to": "/$1",
            },
        },
        {
            "type": "forward-internal",
            "config": {
                "url": os.environ["SAXO_INTERNAL_URL"],
            },
        },
    ],
})

print(json.dumps({"spec": {"policy": {"on_http_request": rules}}}, separators=(",", ":")))
PY
)"

kubectl patch ngroktrafficpolicies.ngrok.k8s.ngrok.com "$POLICY_NAME" \
  -n "$POLICY_NAMESPACE" \
  --context "$CONTEXT" \
  --type=merge \
  -p "$patched_policy"
