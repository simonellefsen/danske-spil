# POC Deployment

This deployment is local-only for Docker Desktop Kubernetes. It keeps `gambler` in observe-only mode and exposes only a ClusterIP service.

The active runtime is Rust. `Dockerfile` builds the full
`danske-spil-gambler` API/UI/worker binary with Dioxus enabled.
`Dockerfile.result-agent` builds the slimmer `danske-spil-result-agent` binary
with `--no-default-features`, so result-agent image builds do not compile the
Dioxus UI dependency graph. Both Dockerfiles first build a tiny stub target from
`Cargo.toml` and `Cargo.lock` to cache dependency compilation, then copy the
real source and rebuild only the application crate. Final images copy only the
binary and CA bundle into `scratch`.

## Components

- `danske-spil-postgres`: CloudNativePG cluster with two instances.
- `gambler-api`: API and web UI. Result-agent API calls are proxied to the `gambler-result-agent` ClusterIP service through `GAMBLER_RESULT_AGENT_URL`.
- `gambler-worker`: scheduled observe-only scanner, paper-placement loop, and settlement-review queue refresher. The default Kubernetes cadence is `GAMBLER_SCAN_INTERVAL_SECONDS=900`, or roughly every 15 minutes.
- `gambler-result-agent`: scheduled paper-only result reconciliation service backed by the separate `danske-spil-result-agent` image. It runs `POST /api/result-agent/run` logic on `GAMBLER_RESULT_AGENT_INTERVAL_SECONDS=900` and exposes the result-agent API routes internally through the `gambler-result-agent` ClusterIP service.
- `hermes-agent`: POC read-only Hermes view backed by the same API image. It does not receive browser control or credentials.

## Deploy

```bash
rtk bash scripts/deploy_local_k8s.sh
```

The script creates `danske-spil-postgres-app` directly in Kubernetes with a generated password if the secret does not already exist. The generated password is not written to the repository.

By default the script builds timestamped local images for the full app and the
result-agent, then patches deployments to those tags so Docker Desktop does not
reuse stale `:local` images.

## Open The Web UI

```bash
rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18080:8080
```

Then open `http://127.0.0.1:18080`.

## Smoke Checks

```bash
rtk kubectl --context docker-desktop -n danske-spil get pods,svc,cluster
rtk kubectl --context docker-desktop -n danske-spil logs deployment/gambler-api --tail=120
rtk kubectl --context docker-desktop -n danske-spil logs deployment/gambler-worker --tail=120
rtk kubectl --context docker-desktop -n danske-spil logs deployment/gambler-result-agent --tail=120
rtk kubectl --context docker-desktop -n danske-spil logs deployment/hermes-agent --tail=120
```

The web UI can trigger a scan, show normalized candidate odds, display structured rationale, create paper-ledger entries, and run the read-only result agent. The API forwards result-agent queue/run requests to the dedicated result-agent service when `GAMBLER_RESULT_AGENT_URL` is configured, and falls back to local execution only for development. The worker runs the scan loop on its configured cadence and advances finished paper bets into the awaiting-result queue. The dedicated result-agent deployment attempts Flashscore public-result discovery for stale rows that do not already have a configured result link. There is no endpoint that submits real bets.
