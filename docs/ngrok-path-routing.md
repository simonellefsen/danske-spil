# ngrok Path Routing

The Docker Desktop cluster uses the ngrok Kubernetes operator and a Google OAuth traffic policy on a shared public hostname:

`https://<shared-ngrok-hostname>`

The shared public endpoint routes to multiple Kubernetes services by combining:

- The public endpoint default upstream for the `danske-spil` app.
- Optional `forward-internal` actions in the existing public endpoint traffic policy when additional backends are added.
- Optional `url-rewrite` actions for apps that are not base-path aware.

## Current POC Routes

- `/danske-spil` routes to `gambler-api.danske-spil:8080`.

The Google OAuth action and allow-list stay in the existing ngrok traffic policy and run before the path routing rules.

## Backend Endpoints

Apply optional internal backend endpoints with:

```bash
rtk kubectl --context docker-desktop apply -f k8s/ngrok/path-backends.yaml
```

Patch the existing Google OAuth traffic policy with path rules:

```bash
rtk bash scripts/patch_ngrok_path_routing.sh
```

The script sets the public endpoint default upstream to `gambler-api.danske-spil:8080`. The `danske-spil` UI is configured with `GAMBLER_BASE_PATH=/danske-spil`, so it can serve its HTML and API under the path prefix without stripping the prefix at ngrok. This avoids consuming an additional ngrok internal endpoint for `danske-spil`.

`scripts/deploy_local_k8s.sh` also runs the patch automatically when the ngrok endpoint and traffic policy resources are present. Set `PATCH_NGROK_PATH_ROUTING=0` before running the deploy script to skip this step.
