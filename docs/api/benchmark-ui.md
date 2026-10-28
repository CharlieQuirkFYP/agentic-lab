# Benchmark dashboard integration

The Go API supports dashboard integration with Pheme. Experiment execution sends configured text cases through Pheme and records `result_source: "measured"`. The API does not select a model runtime; Pheme owns runtime/model selection. The benchmark worker drains Pheme’s buffered metric batches after each experiment and persists them in the Go-owned metrics store for dashboard queries.

[TypeScript request/response types](benchmark-types.ts) are provided as a frontend contract reference.

## Start the backend

From the repository root:

```bash
cd api
cp .env.example .env
go run ./cmd/server
```

Copy `.env.example` once, then configure your local `api/.env`. The server loads `.env` from its working directory automatically, with existing process environment variables taking precedence. Local env files are ignored by Git; restart the API after edits. Missing `.env` files are optional; malformed files fail startup.

The default address is `http://127.0.0.1:8080`. Pheme does not need to be running for benchmark routes. The existing Vite proxy forwards `/api/v1` to this address, so the frontend can use relative URLs. For a separately hosted frontend, configure its exact origin in `API_CORS_ORIGINS` in `.env` and use the backend base URL. Origins have no trailing slash. No cookies/credentials are needed. Allowed methods are GET/POST/OPTIONS; `Content-Disposition` is exposed for downloads. All experiment routes handle preflight.

`API_DATABASE_PATH` defaults to `data/experiments.sqlite` relative to the API process working directory. The directory is created automatically and `api/data/` is ignored by Git. Experiment configurations, results, and separate metric batches survive restart. Use one API process per database. The in-memory queue remains; queued/running experiments found on startup become `failed` with an interruption explanation and completion time. They are not automatically replayed. Completed/failed experiments remain unchanged. Create a new experiment to retry.

## Routes

| Method | Path | Response |
| --- | --- | --- |
| POST | `/api/v1/experiments` | `202` with ID and initial queued status |
| GET | `/api/v1/experiments` | Paginated, filtered experiment history |
| GET | `/api/v1/experiments/:id` | Configuration, current lifecycle status, optional result |
| GET | `/api/v1/experiments/:id/summary` | Generic result provenance and grouped metric observations |
| GET | `/api/v1/experiments/:id/runs` | Paginated runs derived from uploaded metric batches |
| GET | `/api/v1/experiments/:id/metrics` | Raw batches, optionally filtered/paginated |
| POST | `/api/v1/experiments/:id/metrics` | `202` after a validated batch is stored |
| GET | `/api/v1/experiments/:id/export?format=json` | Experiment, summary, runs and raw batches |
| GET | `/api/v1/experiments/:id/export?format=csv` | One row per metric observation with experiment metadata |

All routes below `:id` return `404 {"error":"experiment not found"}` if that experiment does not exist, including metric ingestion. Invalid queries return `400 {"error":"invalid query parameters"}`. Internal errors return a simple error message without storage details. Validation errors from creation/ingestion retain the [experiment](experiments.md) and [metrics](metrics.md) contracts. Successful collections always use arrays, including `[]` for no matches.

### Create and poll

```js
async function request(path, options) {
  const response = await fetch(`/api/v1${path}`, options);
  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.error ?? `HTTP ${response.status}`);
  }
  return response.json();
}

const created = await request('/experiments', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    use_case: 'incident-reporting',
    dataset: 'demo-v1',
    model_config: { name: 'demo-model', runtime: 'mock' },
    environment_profile: { name: 'local-development' },
  }),
});

// Poll while status is queued/running; stop on completed/failed or page unmount.
const experiment = await request(`/experiments/${created.id}`);
const history = await request('/experiments?limit=20&offset=0');
const summary = await request(`/experiments/${created.id}/summary`);
```

The mock may complete before the first poll. Supported statuses are `queued`, `running`, `completed`, `failed`; this is lifecycle progress, not a percentage. A completed experiment is not proof of accurate output. `model_config` and `environment_profile` are recorded request metadata, not verified runtime/device configuration.

### History filters

`GET /api/v1/experiments` accepts:

| Parameter | Meaning |
| --- | --- |
| `limit` | Integer 1–100; default 20 |
| `offset` | Non-negative integer; default 0 |
| `status` | One lifecycle status |
| `use_case` | One currently accepted use-case value |
| `dataset` | Exact dataset identifier |
| `model` | Exact `model_config.name` |
| `created_after` | Inclusive creation-time lower bound; RFC3339 with timezone |
| `created_before` | Inclusive creation-time upper bound; RFC3339 with timezone |

Filters combine with AND. Results sort by creation time descending, then ID descending for ties. `total` counts all matches before pagination. An offset beyond the end returns an empty array with the same total. Empty, repeated, unknown, malformed or inconsistent query parameters return 400. URL-encode values (particularly `+` in timezone offsets). Offset pages can shift when experiments are created; reset to page one on refresh/filter changes.

```json
{
  "experiments": [],
  "total": 0,
  "limit": 20,
  "offset": 0
}
```

The API continues accepting the legacy `interview-assistant` and `license-plate-monitoring` values for compatibility. The dashboard should offer `incident-reporting` as the active scope.

### Summary and data provenance

A completed experiment without uploaded metric batches currently returns:

```json
{
  "experiment_id": "exp_example",
  "status": "completed",
  "result_source": "mock",
  "result": {
    "latency_ms": 123,
    "peak_memory_mb": 256,
    "tokens_per_second": 18.5,
    "success": true
  },
  "run_count": 0,
  "metrics": []
}
```

`result_source` describes only the generic `result`: `mock`, `measured`, or `unavailable`. Show the source label next to the result and do not treat unavailable values as measurements. Summary `result` is null while no result exists; the existing experiment detail omits `result` in that case. Interruption results contain `success: false` and an explanation; their legacy numeric zero fields are not measurements.

Metric observations are reported separately by their producers. Each summary group has this shape:

```json
{
  "name": "transcription_duration_ms",
  "unit": "milliseconds",
  "scope": "run",
  "source": "core.transcription",
  "observation_count": 3,
  "numeric_count": 2,
  "unavailable_count": 1,
  "unavailable_reasons": ["timing unavailable"],
  "min": 100,
  "max": 200,
  "mean": 150
}
```

Only identical `(name, unit, scope, source)` tuples aggregate together. Numeric observations contribute to min/max/arithmetic mean; nulls contribute only to unavailable counts/reasons. String/boolean observations count as observations but are not numerically aggregated; inspect raw events for their values. Min/max/mean are null when no numeric values exist. Ordering is name, unit, scope, source ascending. These are observation statistics, not per-task scores or energy integration. No latency percentiles, accuracy, task success rate, energy per report or endurance values are fabricated.

Render empty `metrics` as “No measurements uploaded”. Render null values as unavailable with their reason; a valid numeric zero remains zero. Power/energy require a verified provider. A `completed` mock experiment can have zero metric runs.

To compare experiments, fetch their details and summaries separately. Show configuration differences and flag differing datasets, model revisions, environments, measurement sources and scopes. No comparison/ranking endpoint currently asserts that experiments are compatible.

### Run and metric drill-down

`GET .../:id/runs?limit=20&offset=0` returns:

```json
{
  "experiment_id": "exp_example",
  "runs": [],
  "total": 0,
  "limit": 20,
  "offset": 0
}
```

Each non-empty run contains `run_id`, `batch_count`, `event_count`, `first_timestamp_ms`, `last_timestamp_ms`, and `metrics` (the summary-group shape above). Runs group batches by `run_id` and sort by that ID ascending. Timestamp bounds include batch and event timestamps; their difference is not an inference latency. This endpoint describes observed metric runs, not invented dataset trials, expected transcripts or quality scores. Real per-sample evaluation will extend the contract when the runner exists.

`GET .../:id/metrics` accepts `run_id`, `name` (exact metric name), `limit` and `offset`. Filters combine with AND. Name filtering keeps only matching events within matching batches. `total` counts matching batches, not events. Batches sort by run ID ascending, then batch sequence ascending. Use the selected run ID and metric name to fetch chart series.

For compatibility, omitting pagination returns all matching batches with `limit: 0` and `offset: 0`. New clients should always send `limit` (1–100); supplying `offset` requires `limit`. Responses contain `experiment_id`, `batches`, `total`, `limit`, `offset`. The first implementation filters and summarizes batches in Go; it is intended for bounded local research datasets rather than high-volume telemetry.

### Populate a metric chart during frontend integration

After creating an experiment, POST this fixture to `/api/v1/experiments/<actual-id>/metrics`:

```json
{
  "schema_version": 1,
  "run_id": "frontend-fixture-001",
  "sequence_number": 0,
  "timestamp_ms": 1792368000000,
  "events": [
    {
      "schema_version": 1,
      "run_id": "frontend-fixture-001",
      "sequence": 0,
      "timestamp_ms": 1792368000000,
      "name": "transcription_duration_ms",
      "value": 150,
      "unit": "milliseconds",
      "scope": "run",
      "source": "frontend_fixture"
    },
    {
      "schema_version": 1,
      "run_id": "frontend-fixture-001",
      "sequence": 1,
      "timestamp_ms": 1792368000000,
      "name": "whole_device_power_watts",
      "value": null,
      "unit": "watts",
      "scope": "device",
      "source": "frontend_fixture",
      "unavailable_reason": "No power instrument configured"
    }
  ]
}
```

This is integration fixture data, not a measurement. The route supplies `experiment_id` when omitted. If present in the batch/events, it must match the route. Identical retries keyed by `(experiment_id, run_id, sequence_number)` return 202 without another stored batch; different content for that identity returns 409. Keep event sequences unique across a run and send each event in one batch; producer-side rebatching of the same event under a different batch identity is not deduplicated. Out-of-order batches are accepted. Invalid batches return 400; uploads over 4 MiB return 413. The Go benchmark worker also imports batches drained from the configured Pheme server after a run.

### Export

`format` defaults to `json`. JSON has `schema_version: 1`, `experiment`, `summary`, `runs` and `batches`, preserving raw unavailable reasons and submitted configuration. CSV repeats experiment/model/environment metadata and the clearly sourced generic result alongside each event; unavailable event values are blank with a reason. If there are no events, CSV still includes one experiment metadata row. Raw event numeric types/values are preserved in JSON; string values in CSV have no additional type column. Text beginning with spreadsheet formula characters is prefixed with an apostrophe in CSV; JSON preserves original text.

```js
const response = await fetch(`/api/v1/experiments/${created.id}/export?format=csv`);
if (!response.ok) throw new Error('Export failed');
const url = URL.createObjectURL(await response.blob());
const link = document.createElement('a');
link.href = url;
link.download = 'experiment.csv';
link.click();
URL.revokeObjectURL(url);
```

The APIs are local development/research endpoints with an explicit browser origin allowlist, not an authenticated multi-user service. Configure `API_ADDR` and appropriate deployment access controls if teammates need network access.
