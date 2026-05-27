# ngrok Path Routing

The Docker Desktop cluster uses the ngrok Kubernetes operator and a Google OAuth traffic policy on a shared public hostname.

`https://<shared-ngrok-hostname>`

The shared gateway is now owned by the local repository at:

`/Users/lindau/codex/shared-ngrok-gateway`

This `danske-spil` repository owns only the app namespace, workloads, services, and base-path-aware UI behavior. It must not patch or apply the shared public ngrok `AgentEndpoint` or `NgrokTrafficPolicy`.

## Current POC Routes

- `/danske-spil` routes to `gambler-api.danske-spil:8080`.

The Google OAuth action and allow-list stay in the shared gateway traffic policy and run before route forwarding.

## App Responsibilities

- Keep `gambler-api` healthy in namespace `danske-spil`.
- Keep `GAMBLER_BASE_PATH=/danske-spil` configured so the UI and API work under the shared path prefix.
- Verify the service with a local port-forward before changing shared gateway routes.
- Request shared gateway route changes in `/Users/lindau/codex/shared-ngrok-gateway`.

## Local Verification

Deploy this app:

```bash
rtk bash scripts/deploy_local_k8s.sh
```

Port-forward the app service:

```bash
rtk kubectl --context docker-desktop -n danske-spil port-forward svc/gambler-api 18080:8080
```

Check the base-path route locally:

```bash
rtk curl -sS http://127.0.0.1:18080/danske-spil/healthz
```

## Shared Gateway Handoff

After this app is healthy, apply or update the shared gateway from its own repository:

```bash
cd /Users/lindau/codex/shared-ngrok-gateway
make status
make render
make apply
```

The shared gateway repository is the source of truth for the public hostname, Google SSO configuration, and cross-app path routing.
