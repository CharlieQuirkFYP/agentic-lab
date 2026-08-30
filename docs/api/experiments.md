# Experiment API

The experiment API starts generic benchmark runs asynchronously. The current backend stores experiments in memory, uses an in-memory queue, and executes a mocked runner in a background Go worker.

## Create Experiment

```http
POST /api/v1/experiments
```

Returns `202 Accepted` after the experiment is stored as `queued` and enqueued for background execution. The request does not wait for benchmark execution to finish.

Supported `use_case` values:

* `incident-reporting`
* `interview-assistant`
* `license-plate-monitoring`

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
