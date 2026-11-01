# Agentic Lab benchmark web

React + TypeScript + Vite frontend for the benchmark dashboard, JSON test runner, metrics view, and download manager.

## Development

```sh
npm ci
npm run dev
```

Start the Go API from `api/`:

```sh
API_CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 go run ./cmd/server
```

Vite proxies `/api/v1` to `http://127.0.0.1:8080`. Set `VITE_API_BASE_URL` when the API is hosted elsewhere.

## Backend contract

The frontend uses the benchmark API directly:

- `GET /api/v1/experiments` lists experiments.
- `POST /api/v1/experiments` queues a JSON benchmark configuration.
- `GET /api/v1/experiments/:id/summary` returns aggregate results and metric summaries.
- `GET /api/v1/experiments/:id/metrics` returns stored JSON metric batches.
- `GET /api/v1/experiments/:id/export?format=json` downloads the complete JSON export.

The dashboard does not fabricate benchmark measurements. Empty backend data is shown as an empty state, and unavailable metrics remain unavailable.

## Verification

```sh
npm run build
npm test
```
