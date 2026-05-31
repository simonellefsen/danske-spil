#!/usr/bin/env bash
set -euo pipefail

CONTEXT="${KUBE_CONTEXT:-docker-desktop}"
NAMESPACE="${NAMESPACE:-danske-spil}"
IMAGE="${IMAGE:-danske-spil-gambler:$(date +%Y%m%d%H%M%S)}"
RESULT_AGENT_IMAGE="${RESULT_AGENT_IMAGE:-$IMAGE}"
BUILD_PROFILE="${BUILD_PROFILE:-k8s-dev}"
DEPLOY_SCOPE="${DEPLOY_SCOPE:-auto}"
SECRET_NAME="danske-spil-postgres-app"

kubectl --context "$CONTEXT" get namespace "$NAMESPACE" >/dev/null 2>&1 || \
  kubectl --context "$CONTEXT" create namespace "$NAMESPACE"

if ! kubectl --context "$CONTEXT" -n "$NAMESPACE" get secret "$SECRET_NAME" >/dev/null 2>&1; then
  PASSWORD="$(openssl rand -base64 32)"
  kubectl --context "$CONTEXT" -n "$NAMESPACE" create secret generic "$SECRET_NAME" \
    --from-literal=username=danske_spil \
    --from-literal=password="$PASSWORD"
fi

docker build --build-arg "BUILD_PROFILE=$BUILD_PROFILE" -t "$IMAGE" .
if [ "$RESULT_AGENT_IMAGE" != "$IMAGE" ]; then
  docker tag "$IMAGE" "$RESULT_AGENT_IMAGE"
fi

if [ "$DEPLOY_SCOPE" = "auto" ]; then
  if kubectl --context "$CONTEXT" -n "$NAMESPACE" get cluster/danske-spil-postgres >/dev/null 2>&1 \
    && kubectl --context "$CONTEXT" -n "$NAMESPACE" get deployment/gambler-api >/dev/null 2>&1 \
    && kubectl --context "$CONTEXT" -n "$NAMESPACE" get deployment/gambler-worker >/dev/null 2>&1 \
    && kubectl --context "$CONTEXT" -n "$NAMESPACE" get deployment/gambler-result-agent >/dev/null 2>&1 \
    && kubectl --context "$CONTEXT" -n "$NAMESPACE" get deployment/hermes-agent >/dev/null 2>&1; then
    DEPLOY_SCOPE="app"
  else
    DEPLOY_SCOPE="full"
  fi
fi

case "$DEPLOY_SCOPE" in
  app)
    kubectl --context "$CONTEXT" apply -f k8s/base/20-gambler.yaml
    kubectl --context "$CONTEXT" apply -f k8s/base/30-hermes-poc.yaml
    ;;
  full)
    kubectl --context "$CONTEXT" apply -f k8s/base
    ;;
  *)
    echo "DEPLOY_SCOPE must be one of: auto, app, full" >&2
    exit 2
    ;;
esac

kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/gambler-api gambler-api="$IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/gambler-worker gambler-worker="$IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/gambler-result-agent gambler-result-agent="$RESULT_AGENT_IMAGE"
kubectl --context "$CONTEXT" -n "$NAMESPACE" set image deployment/hermes-agent hermes-agent="$IMAGE"
if [ "$DEPLOY_SCOPE" = "full" ]; then
  kubectl --context "$CONTEXT" -n "$NAMESPACE" wait --for=condition=Ready cluster/danske-spil-postgres --timeout=300s
fi
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/gambler-api --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/gambler-worker --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/gambler-result-agent --timeout=180s
kubectl --context "$CONTEXT" -n "$NAMESPACE" rollout status deployment/hermes-agent --timeout=180s

kubectl --context "$CONTEXT" -n "$NAMESPACE" get pods,svc,cluster
