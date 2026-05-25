#!/usr/bin/env bash
set -euo pipefail

CONTEXT="${KUBE_CONTEXT:-docker-desktop}"
POLICY_NAMESPACE="${POLICY_NAMESPACE:-saxo-rust}"
POLICY_NAME="${POLICY_NAME:-daytrader-oauth}"

expressions="$(kubectl get ngroktrafficpolicies.ngrok.k8s.ngrok.com "$POLICY_NAME" \
  -n "$POLICY_NAMESPACE" \
  --context "$CONTEXT" \
  -o jsonpath='{.spec.policy.on_http_request[*].expressions}')"

if [[ "$expressions" == *"/danske-spil"* && "$expressions" == *"/saxo-daytrader"* ]]; then
  echo "ngrok path routing already present"
  exit 0
fi

kubectl patch ngroktrafficpolicies.ngrok.k8s.ngrok.com "$POLICY_NAME" \
  -n "$POLICY_NAMESPACE" \
  --context "$CONTEXT" \
  --type=json \
  -p '[
    {
      "op": "add",
      "path": "/spec/policy/on_http_request/3",
      "value": {
        "expressions": ["req.url.path.startsWith(\"/danske-spil\")"],
        "actions": [
          {
            "type": "forward-internal",
            "config": {
              "url": "http://danske-spil-gambler.internal:80"
            }
          }
        ]
      }
    },
    {
      "op": "add",
      "path": "/spec/policy/on_http_request/4",
      "value": {
        "expressions": ["req.url.path.startsWith(\"/saxo-daytrader\")"],
        "actions": [
          {
            "type": "url-rewrite",
            "config": {
              "from": "/saxo-daytrader/?(.*)",
              "to": "/$1"
            }
          },
          {
            "type": "forward-internal",
            "config": {
              "url": "http://saxo-daytrader.internal:80"
            }
          }
        ]
      }
    }
  ]'
