# POC Deployment

This deployment is local-only for Docker Desktop Kubernetes. It keeps `gambler` in observe-only mode and exposes only a ClusterIP service.

## Components

- `danske-spil-postgres`: CloudNativePG cluster with two instances.
- `gambler-api`: API and web UI.
- `gambler-worker`: scheduled observe-only scanner loop.
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

The web UI can trigger a scan, show normalized candidate odds, display structured rationale, and create paper-ledger entries. There is no endpoint that submits real bets.
