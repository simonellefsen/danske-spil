# Kubernetes Architecture

Target cluster: local Docker Desktop Kubernetes.

Target namespace: `danske-spil`.

## Workloads

- `gambler-api`: future internal API and browser-observation service.
- `gambler-worker`: future scheduled observation and strategy scoring worker.
- `gambler-mcp`: future Hermes-safe MCP adapter.
- `hermes-agent`: Hermes gateway and reflection engine.
- `hermes-weekly-reflection`: suspended CronJob until configured.
- `danske-spil-postgres`: CloudNativePG cluster with two instances.

## Storage

- `danske-spil-postgres`: CNPG PVCs for database state.
- `hermes-data`: PVC mounted at `/opt/data`.
- Optional browser profile PVC should be encrypted or avoided until the session model is understood.

## Secrets

Separate secrets by blast radius:

- `gambler-env`: Danske Spil credentials and browser config. Only `gambler` workloads may mount it.
- `hermes-env`: Hermes/model/API keys. Must not include Danske Spil credentials.
- `danske-spil-postgres-app`: database username, password, and URL.

## CNPG Skeleton

The future manifest should use two CNPG instances:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: danske-spil-postgres
  namespace: danske-spil
spec:
  instances: 2
  bootstrap:
    initdb:
      database: danske_spil
      owner: danske_spil
      secret:
        name: danske-spil-postgres-app
  storage:
    size: 5Gi
```

## Network Policy Direction

Start closed and open narrowly:

- `hermes-agent` can call `gambler-mcp`.
- `gambler-mcp` can call Postgres and `gambler-api`.
- `gambler-api` and `gambler-worker` can call Postgres and `danskespil.dk`.
- No public ingress in the first deployment.

## Deployment Order

1. Namespace and secrets.
2. CNPG operator if not already installed.
3. Postgres cluster and app secret.
4. `gambler` read-only service.
5. `gambler-mcp`.
6. Hermes with MCP wait/init check.
7. Suspended CronJob for manual smoke tests.

## Smoke Tests

```bash
rtk kubectl --context docker-desktop -n danske-spil get pods,svc,pvc,cluster
rtk kubectl --context docker-desktop -n danske-spil logs deployment/gambler-api --tail=120
rtk kubectl --context docker-desktop -n danske-spil logs deployment/hermes-agent --tail=120
```
