#!/usr/bin/env bash
set -euo pipefail

CONTEXT="${KUBE_CONTEXT:-docker-desktop}"
NAMESPACE="${NAMESPACE:-danske-spil}"
IMAGE="${IMAGE:-danske-spil-gambler:$(date +%Y%m%d%H%M%S)}"
SECRET_NAME="danske-spil-postgres-app"
PATCH_NGROK_PATH_ROUTING="${PATCH_NGROK_PATH_ROUTING:-1}"

kubectl --context "$CONTEXT" get namespace "$NAMESPACE" >/dev/null 2>&1 || \
  kubectl --context "$CONTEXT" create namespace "$NAMESPACE"

if ! kubectl --context "$CONTEXT" -n "$NAMESPACE" get secret "$SECRET_NAME" >/dev/null 2>&1; then
  PASSWORD="$(openssl rand -base64 32)"
  kubectl --context "$CONTEXT" -n "$NAMESPACE" create secret generic "$SECRET_NAME" \
    --from-literal=username=danske_spil \
    --from-literal=password="$PASSWORD"
fi

docker build -t "$IMAGE" .

kubectl --context "$CONTEXT" apply -f k8s/base
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/gambler-api gambler-api="$IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/gambler-worker gambler-worker="$IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/hermes-agent hermes-agent="$IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" wait --for=condition=Ready cluster/danske-spil-postgres --timeout=300s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/gambler-api --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/gambler-worker --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/hermes-agent --timeout=180s

if [ "$PATCH_NGROK_PATH_ROUTING" = "1" ]; then
  POLICY_NAMESPACE="${POLICY_NAMESPACE:-saxo-rust}"
  POLICY_NAME="${POLICY_NAME:-daytrader-oauth}"
  PUBLIC_ENDPOINT_NAME="${PUBLIC_ENDPOINT_NAME:-daytrader-frontend}"

  if kubectl --context "$CONTEXT" -n "$POLICY_NAMESPACE" get agentendpoints.ngrok.k8s.ngrok.com "$PUBLIC_ENDPOINT_NAME" >/dev/null 2>&1 \
    && kubectl --context "$CONTEXT" -n "$POLICY_NAMESPACE" get ngroktrafficpolicies.ngrok.k8s.ngrok.com "$POLICY_NAME" >/dev/null 2>&1; then
    KUBE_CONTEXT="$CONTEXT" \
      POLICY_NAMESPACE="$POLICY_NAMESPACE" \
      POLICY_NAME="$POLICY_NAME" \
      PUBLIC_ENDPOINT_NAME="$PUBLIC_ENDPOINT_NAME" \
      bash scripts/patch_ngrok_path_routing.sh
  else
    echo "ngrok path routing resources not found; skipping patch"
  fi
fi

kubectl --context "$CONTEXT" -n "$NAMESPACE" get pods,svc,cluster
