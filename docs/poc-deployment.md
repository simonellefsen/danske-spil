# POC Deployment

This deployment is local-only for Docker Desktop Kubernetes. It keeps `gambler` in observe-only mode and exposes only a ClusterIP service.

The active runtime is Rust. `Dockerfile` builds one shared scratch image that
contains both `/gambler` and `/result-agent`. The API, worker, and Hermes
deployments run `/gambler`; the result-agent deployment runs `/result-agent`
from the same image. This keeps the result agent as a separate Kubernetes
process while avoiding a second full Docker/Rust compile during local deploys.
The Dockerfile uses BuildKit cache mounts for Cargo registry, git, and target
state, first building tiny stub targets from `Cargo.toml` and `Cargo.lock`, then
copying the real source and rebuilding the application binaries. The legacy
`Dockerfile.result-agent` remains available for isolated result-agent image
experiments, but the normal Makefile and deploy script no longer use it.

## Components

- `danske-spil-postgres`: CloudNativePG cluster with two instances.
- `gambler-api`: API and web UI. Result-agent API calls are proxied to the `gambler-result-agent` ClusterIP service through `GAMBLER_RESULT_AGENT_URL`.
- `gambler-worker`: scheduled observe-only scanner, paper-placement loop, and settlement-review queue refresher. The default Kubernetes cadence is `GAMBLER_SCAN_INTERVAL_SECONDS=900`, or roughly every 15 minutes.
- `gambler-result-agent`: scheduled paper-only result reconciliation service backed by the shared scratch image and `command: ["/result-agent"]`. It runs `POST /api/result-agent/run` logic on `GAMBLER_RESULT_AGENT_INTERVAL_SECONDS=900` and exposes the result-agent API routes internally through the `gambler-result-agent` ClusterIP service.
- `hermes-agent`: POC read-only Hermes loop backed by the same API image. It runs `/gambler hermes-agent`, serves the Hermes API, refreshes paper-only reflections on `HERMES_REFLECTION_INTERVAL_SECONDS`, and does not receive browser control or credentials.

## Deploy

```bash
rtk bash scripts/deploy_local_k8s.sh
```

Or use the Makefile wrapper:

```bash
rtk make k8s-deploy
```

The script creates `danske-spil-postgres-app` directly in Kubernetes with a generated password if the secret does not already exist. The generated password is not written to the repository.

By default the script builds one timestamped local image with both binaries,
then patches all deployments to that tag so Docker Desktop does not reuse stale
`:local` images. `RESULT_AGENT_IMAGE` defaults to the same image tag; when set
to a different tag the script retags the already-built shared image instead of
running a second Docker build.

`rtk make docker-build` builds the shared scratch-container image without
applying Kubernetes manifests. `rtk make k8s-status` prints the current local
namespace pods, deployments, services, and CNPG cluster state.

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
