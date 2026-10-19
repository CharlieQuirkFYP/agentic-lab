# Experiment API

The experiment API starts generic benchmark runs asynchronously. The current backend stores experiments in memory, uses an in-memory queue, and executes a mocked runner in a background Go worker.

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

`GET /api/v1/experiments/:id/metrics` returns stored batches for dashboard inspection. Metrics are currently held in a separate in-memory repository and are not merged into `Experiment.Result`; persistence and listing/filtering are later work. See the [metrics contract](metrics.md) for the Rust pub/sub and host-exporter path.

## Current Limitations and Planned Work

The runner returns hardcoded values; latency, memory, and tokens per second are not actual benchmark measurements. Experiments and Go-side metric batches are lost when the process restarts. There is no experiment listing/filtering or export endpoint or persistent result storage, and no measured resource integration in `Experiment.Result` or verified whole-device power/energy telemetry.

The [planned architecture](../architecture.md) adds a real incident scenario runner, persistent experiment storage, dashboard query APIs, and measured results. The separate Rust Pheme VA metrics path can currently emit application/resource telemetry (with additional GPU, component-temperature, and single-battery values on supported Linux hosts), but that telemetry is not integrated into `Experiment.Result`; verified whole-device power and energy are not currently available. These benchmark-result capabilities are not available in the current API.
