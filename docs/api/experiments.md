# Experiment API

The experiment API starts generic benchmark runs asynchronously. The server stores experiments and separate metric batches in a Go-owned SQLite database, uses an in-memory queue, and executes a mocked runner in a background Go worker. See the [benchmark UI integration guide](benchmark-ui.md) for history/filtering, summary/run responses, export, browser configuration and frontend examples.

## Create Experiment

```http
POST /api/v1/experiments
```

Returns `202 Accepted` after the experiment is stored as `queued` and enqueued for background execution. The request does not wait for benchmark execution to finish.

Currently accepted `use_case` values:

- `incident-reporting`
- `interview-assistant`
- `license-plate-monitoring`

These values describe the existing implementation. The active project focuses on incident reporting; interview development is deferred and licence-plate monitoring has been removed from project scope. The legacy API value remains accepted until the planned code/test change is implemented. See the [requirements](../requirements.md) and [roadmap](../roadmap.md).

Example request:

```json
{
  "use_case": "incident-reporting",
  "dataset": "sample-incidents",
  "model_config": {
    "name": "mock-model",
    "version": "0.1",
    "quantization": "q4",
    "runtime": "mock"
  },
  "environment_profile": {
    "name": "local-dev",
    "type": "developer-machine",
    "cpu_cores": 8,
    "memory_mb": 16384,
    "gpu_info": "integrated"
  }
}
```

Example response:

```json
{
  "id": "exp_...",
  "status": "queued"
}
```

Invalid or unsupported `use_case` values return `400 Bad Request`.

## Get Experiment

```http
GET /api/v1/experiments/:id
```

Returns `200 OK` with the current experiment state when found, or `404 Not Found` for an unknown experiment ID.

Status values:

- `queued`
- `running`
- `completed`
- `failed`

Completed experiments include a generic benchmark result with latency, peak memory, optional tokens per second, success, and optional error message fields.

## Append Metrics

The Rust host and future benchmark workers can append detailed measurements without changing the generic result shape:

```http
POST /api/v1/experiments/:id/metrics
Content-Type: application/json
```

The request body is a versioned batch containing a `run_id`, batch sequence, timestamp, and scalar metric events. Events carry their own name, value, unit, scope, source, timestamp, and sequence. Numeric, string, boolean, and explicit unavailable (`value: null` plus `unavailable_reason`) values are supported. The route validates that supplied envelope/event experiment IDs agree with `:id` and returns `202 Accepted` after storing the batch.

Example:

```json
{
  "schema_version": 1,
  "experiment_id": "exp_...",
  "run_id": "run_...",
  "sequence_number": 0,
  "timestamp_ms": 1760000000000,
  "events": [
    {
      "schema_version": 1,
      "run_id": "run_...",
      "sequence": 0,
      "timestamp_ms": 1760000000000,
      "name": "end_to_end_request_duration_ms",
      "value": 742.4,
      "unit": "milliseconds",
      "scope": "run",
      "source": "core.workflow"
    }
  ]
}
```

`GET /api/v1/experiments/:id/metrics` returns stored batches for dashboard inspection, with optional `run_id`, `name`, `limit` and `offset` parameters. Batches remain separate from `Experiment.Result`. Unknown experiments return 404 for all experiment detail/metrics/export routes. Identical metric-batch retries are idempotent; conflicting retries return 409. See the [metrics contract](metrics.md) for the Rust pub/sub and host-exporter path.

## Dashboard queries

- `GET /api/v1/experiments`: paginated history; exact status/use-case/dataset/model and inclusive creation-time filters.
- `GET /api/v1/experiments/:id/summary`: generic result with provenance and grouped reported metric statistics.
- `GET /api/v1/experiments/:id/runs`: paginated metric-run summaries; no synthetic sample trials.
- `GET /api/v1/experiments/:id/export?format=json|csv`: raw observations and experiment metadata.

New experiments include `result_source: "mock"` in detail/history responses. The server defaults to `data/experiments.sqlite` relative to its working directory (`API_DATABASE_PATH` overrides it). Completed results survive restarts. Previously queued/running experiments become failed with an interruption explanation; the volatile queue is not replayed. In-memory repositories remain available for isolated tests/development.

## Current Limitations and Planned Work

The runner returns hardcoded values; latency, memory, and tokens per second are not actual benchmark measurements. There is no real scenario runner, automatic Pheme metric import, measured resource integration in `Experiment.Result`, or verified whole-device power/energy telemetry. Summary metrics reflect uploaded observations and do not turn the mock generic result into a measurement.

The [planned architecture](../architecture.md) adds a real incident scenario runner and measured workflow results. The separate Rust Pheme VA metrics path can emit application/resource telemetry (with additional GPU, component-temperature, and single-battery values on supported Linux hosts), but verified whole-device power and energy are not currently available.
