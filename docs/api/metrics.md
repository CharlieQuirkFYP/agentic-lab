# Metrics API

Status: implemented for the Rust `pheme-va` workspace and the initial Go API. The Go repository is in memory for now; it is separate from `benchmark.Experiment.Result` and will be replaceable with SQLite later.

## Architecture

The Rust `metrics` crate is the internal event bus and wire-contract boundary:

```text
core publishes typed events
          |
          v
metrics::MetricsHub
   |       |       |
   v       v       v
 TUI    batcher  test/local sink
                   |
                   v
          host exporter / FFI drain
                   |
                   v
        Go metrics HTTP endpoint
```

`metrics` does not depend on Go, HTTP, a UI, or `core`. `core` depends on `metrics` and publishes application events through a per-run `MetricsContext`. This avoids a dependency cycle and allows iOS/Android hosts to provide their own resource samplers.

## Rust application metrics

The instrumented `Engine` publishes these events when the run configuration has `enabled: true`:

| Event name                          | Unit                           | Scope | Source               |
| ----------------------------------- | ------------------------------ | ----- | -------------------- |
| `audio_normalization_duration_ms`   | `milliseconds`                 | `run` | `core.audio`         |
| `speech_gate_duration_ms`           | `milliseconds`                 | `run` | `core.audio`         |
| `whisper_transcription_duration_ms` | `milliseconds`                 | `run` | `core.transcription` |
| `end_to_end_request_duration_ms`    | `milliseconds`                 | `run` | `core.workflow`      |
| `retry_count`                       | `count`                        | `run` | `core.engine`        |
| `transcript_status`                 | `status`                       | `run` | `core.engine`        |
| `workflow_outcome`                  | `status` (`success`/`failure`) | `run` | `core.workflow`      |

`whisper_transcription_duration_ms` is emitted once per recognizer attempt, so a retry produces two attempt timings and one `retry_count` event. The end-to-end timer is the primary latency measure; stage timings are diagnostic.

Incident analysis remains separate from transcription. `core::analyze_with_metrics` wraps the existing pure transcript-to-report `IncidentAnalyzer` and publishes `incident_analysis_duration_ms`. That event is marked incident-specific and is filtered unless `MetricsConfig.incident_active` is true. The analyzer still accepts text and returns an `IncidentReport`; no combined workflow endpoint was introduced.

Example configuration:

```rust
let config = metrics::MetricsConfig {
    enabled: true,
    incident_active: true,
    resource_sampling: true,
};
let run = metrics::MetricsContext::new(
    "run-123",
    Some("exp-123".to_owned()),
    config,
    hub,
);
let result = engine.transcribe_with_metrics(audio, run);
```

## Resource metrics

`SysinfoResourceSampler` is the initial Linux/macOS/Windows provider. The `metrics` crate enables its `desktop` feature by default; `core` and FFI use the generic crate with default features disabled so mobile builds do not pull in desktop process APIs. It records:

- process CPU percentage (`process_cpu_percent`, process scope)
- system CPU percentage (`system_cpu_percent`, system scope)
- process RAM bytes (`ram_usage_bytes`, process scope)
- system RAM bytes (`ram_usage_bytes`, system scope)

The following fields exist in the schema and are emitted as `value: null` with an `unavailable_reason` until a host-specific provider supplies them:

- `gpu_usage_percent`
- `temperature_celsius`
- `battery_drain_percent`
- `whole_device_power_watts`
- `energy_joules`
- `energy_watt_hours`

This is intentional. CPU percentage is not power, and unsupported power/energy/thermal values must not be represented as zero. `ResourceSampler` is the extension point for native iOS/Android measurements. `EnergyAccumulator` integrates available whole-device power samples with the trapezoidal rule and resets across unavailable intervals.

The Rust development server samples resources before and after a request when started with `--metrics-enabled --resource-sampling`. It does not claim that desktop `sysinfo` can measure every device sensor.

## Event and batch contract

Every event contains:

```json
{
  "schema_version": 1,
  "experiment_id": "exp-123",
  "run_id": "run-123",
  "sequence": 7,
  "timestamp_ms": 1760000000000,
  "name": "process_cpu_percent",
  "value": 23.4,
  "unit": "percent",
  "scope": "process",
  "source": "sysinfo"
}
```

An unavailable value is explicit:

```json
{
  "name": "whole_device_power_watts",
  "value": null,
  "unit": "watts",
  "scope": "device",
  "source": "sysinfo",
  "unavailable_reason": "not provided by the desktop sampler"
}
```

A batch groups events for one run:

```json
{
  "schema_version": 1,
  "experiment_id": "exp-123",
  "run_id": "run-123",
  "sequence_number": 0,
  "timestamp_ms": 1760000000000,
  "events": [/* metric events */]
}
```

The event sequence is run-local and monotonically assigned. The batch sequence is assigned by the local batcher. A host exporter should retry a batch using its run and batch sequence as an idempotency key once persistent storage is added.

## Go API

### Append a batch

```http
POST /api/v1/experiments/:id/metrics
Content-Type: application/json
```

The route identifier is authoritative. `experiment_id` may be omitted from the envelope and is filled from `:id`; if supplied, it must match. The batch run ID, event run IDs, event sequences, schema version, scalar values, and unavailable-value reasons are validated before storage.

A successful append returns `202 Accepted`:

```json
{
  "status": "accepted",
  "event_count": 7
}
```

Malformed or inconsistent batches return `400`. The current storage is process-local memory and is lost on restart.

### Read batches

```http
GET /api/v1/experiments/:id/metrics
```

Returns:

```json
{
  "experiment_id": "exp-123",
  "batches": [/* stored batches */]
}
```

This read endpoint is for development/dashboard inspection. It does not merge metrics into the generic experiment result.

## Rust development-host drain

The Rust server exposes:

```http
GET /v1/metrics/batches
```

It drains the local batcher and returns a JSON array of batches. A host-level exporter can POST each returned batch to the Go route, for example:

```bash
curl -s http://127.0.0.1:8000/v1/metrics/batches
curl -X POST http://127.0.0.1:8080/api/v1/experiments/exp-123/metrics \
  -H 'Content-Type: application/json' \
  --data-binary @batch.json
```

Use `X-Run-ID` and `X-Experiment-ID` headers on Rust requests to control correlation IDs. `X-Incident-Active: true|false` overrides the server's incident-metrics setting for a request.

## Mobile/FFI path

When built with the `whisper` feature, the C ABI exposes:

- `pheme_va_metrics_set_enabled` to toggle future event collection
- `pheme_va_metrics_drain` to receive JSON batches as an owned C string

The native Swift/Kotlin host owns HTTP transport and can forward each drained batch to the same Go endpoint. Native platform code should implement `ResourceSampler` in a future host integration rather than adding iOS/Android APIs to the portable metrics crate.
