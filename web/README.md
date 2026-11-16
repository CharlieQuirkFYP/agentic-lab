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

Phone metrics start with the current/latest run, followed by the full selected Metrics overview and conversation/sweep/run history. `GET /api/v1/phone/overview` supplies full-scope totals independently of the activity page. URL filters survive opening and returning from a detail view. Legacy uploads retain unknown conversation ownership and outcomes; explicit installation-scoped IDs alone connect historical runs.

JSON, CSV and Markdown reports fetch every page of `/phone/observations` and `/phone/legacy-observations` at one API snapshot. Aggregation and serializers come from `shared/phone-benchmarks/`, also used by the phone. Load an analysis snapshot to inspect raw sample histories and eligible execution-duration statistics. New server receipts update the overview; previously exported reports retain their snapshot identity. See [the phone API contract](../docs/api/phone-observations-api.md).

## Verification

```sh
npm run build
npm test
npm run lint
```
