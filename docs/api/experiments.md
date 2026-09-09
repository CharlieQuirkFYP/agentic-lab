# Experiment API

The experiment API starts generic benchmark runs asynchronously. The current backend stores experiments in memory, uses an in-memory queue, and executes a mocked runner in a background Go worker.

## Create Experiment

```http
POST /api/v1/experiments
```

Returns `202 Accepted` after the experiment is stored as `queued` and enqueued for background execution. The request does not wait for benchmark execution to finish.

Currently accepted `use_case` values:

* `incident-reporting`
* `interview-assistant`
* `license-plate-monitoring`

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

* `queued`
* `running`
* `completed`
* `failed`

Completed experiments include a generic benchmark result with latency, peak memory, optional tokens per second, success, and optional error message fields.

## Current Limitations and Planned Work

The runner returns hardcoded values; latency, memory, and tokens per second are not actual measurements. Experiments are lost when the process restarts. There is no experiment listing/filtering or export endpoint, persistent result storage, or power/energy/thermal telemetry.

The [planned architecture](../architecture.md) adds a real incident scenario runner, persistent experiment storage, dashboard query APIs, and measured results. Those capabilities are not available in the current API.
