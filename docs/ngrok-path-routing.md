# ngrok Path Routing

The Docker Desktop cluster uses the ngrok Kubernetes operator and a Google OAuth traffic policy on:

`https://unground-uncraftily-vivienne.ngrok-free.dev`

The shared public endpoint routes to multiple Kubernetes services by combining:

- The public endpoint default upstream for the `danske-spil` app.
- One internal `AgentEndpoint` for the `saxo-daytrader` backend.
- `forward-internal` actions in the existing public endpoint traffic policy.
- Optional `url-rewrite` actions for apps that are not base-path aware.

## Current POC Routes

- `/saxo-daytrader` routes to `daytrader-frontend.saxo-rust:8000`.
- `/danske-spil` routes to `gambler-api.danske-spil:8080`.

The Google OAuth action and allow-list stay in the existing ngrok traffic policy and run before the path routing rules.

## Backend Endpoints

Apply the internal backend endpoint with:

```bash
rtk kubectl --context docker-desktop apply -f k8s/ngrok/path-backends.yaml
```

Patch the existing Google OAuth traffic policy with path rules:

```bash
rtk bash scripts/patch_ngrok_path_routing.sh
```

The script sets the public endpoint default upstream to `gambler-api.danske-spil:8080`. The `danske-spil` UI is configured with `GAMBLER_BASE_PATH=/danske-spil`, so it can serve its HTML and API under the path prefix without stripping the prefix at ngrok. This avoids consuming an additional ngrok internal endpoint for `danske-spil`.

The `saxo-daytrader` route strips `/saxo-daytrader` before forwarding because that frontend is served from `/`.
