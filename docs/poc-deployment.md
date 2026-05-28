# POC Deployment

This deployment is local-only for Docker Desktop Kubernetes. It keeps `gambler` in observe-only mode and exposes only a ClusterIP service.

The active runtime is a Rust binary. The Dockerfile first builds a tiny stub
binary from `Cargo.toml` and `Cargo.lock` to cache dependency compilation, then
copies the real source and rebuilds the application crate. The final image still
copies only the binary and CA bundle into `scratch`. The first build after
Docker cache eviction is still slow, but source-only edits should reuse compiled
dependencies and mainly rebuild `danske-spil-gambler`.

## Components

- `danske-spil-postgres`: CloudNativePG cluster with two instances.
- `gambler-api`: API and web UI.
- `gambler-worker`: scheduled observe-only scanner and result-agent loop. The default Kubernetes cadence is `GAMBLER_SCAN_INTERVAL_SECONDS=900`, or roughly every 15 minutes.
- `hermes-agent`: POC read-only Hermes view backed by the same API image. It does not receive browser control or credentials.

## Deploy

```bash
rtk bash scripts/deploy_local_k8s.sh
```

The script creates `danske-spil-postgres-app` directly in Kubernetes with a generated password if the secret does not already exist. The generated password is not written to the repository.

By default the script builds a timestamped local image and patches the deployments to that tag so Docker Desktop does not reuse a stale `:local` image.

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
rtk kubectl --context docker-desktop -n danske-spil logs deployment/hermes-agent --tail=120
```

The web UI can trigger a scan, show normalized candidate odds, display structured rationale, create paper-ledger entries, and run the read-only result agent. The worker also runs the scan loop on its configured cadence, advances finished paper bets into the awaiting-result queue, and attempts Flashscore public-result discovery for stale rows that do not already have a configured result link. There is no endpoint that submits real bets.
