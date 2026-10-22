# Metrics API

Status: implemented for the Rust `pheme-va` workspace and the Go API. The Go server persists metric batches in its experiment SQLite database, separate from `benchmark.Experiment.Result`. An in-memory implementation remains for tests. See the [dashboard integration guide](benchmark-ui.md) for query and summary contracts.

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

| Event name                             | Unit                           | Scope | Source               |
| -------------------------------------- | ------------------------------ | ----- | -------------------- |
| `audio_normalization_duration_ms`      | `milliseconds`                 | `run` | `core.audio`         |
| `speech_gate_duration_ms`              | `milliseconds`                 | `run` | `core.audio`         |
| `transcription_duration_ms`            | `milliseconds`                 | `run` | `core.transcription` |
| `model_feature_extraction_duration_ms` | `milliseconds`                 | `run` | `core.transcription` |
| `model_inference_duration_ms`          | `milliseconds`                 | `run` | `core.transcription` |
| `model_decoding_duration_ms`           | `milliseconds`                 | `run` | `core.transcription` |
| `model_id`                             | `status`                       | `run` | `core.transcription` |
| `model_family`                         | `status`                       | `run` | `core.transcription` |
| `model_revision`                       | `status`                       | `run` | `core.transcription` |
| `end_to_end_request_duration_ms`       | `milliseconds`                 | `run` | `core.workflow`      |
| `retry_count`                          | `count`                        | `run` | `core.engine`        |
| `transcript_status`                    | `status`                       | `run` | `core.engine`        |
| `workflow_outcome`                     | `status` (`success`/`failure`) | `run` | `core.workflow`      |

`transcription_duration_ms` is emitted once per recognizer attempt, regardless of whether the selected model is Whisper, Zipformer, or another adapter. The model metadata identifies the selected model for the run. Model-specific stage timings are emitted when an adapter supplies them; otherwise the event has `value: null` and an explanation. A retry produces two attempt timings and one `retry_count` event. The end-to-end timer is the primary latency measure; stage timings are diagnostic. The legacy `whisper_transcription_duration_ms` name remains available only when a caller explicitly starts the deprecated `Stage::WhisperTranscription`.

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

The current desktop provider may also expose component temperature. Its Linux extension can expose peak utilisation across readable DRM cards and signed single-battery capacity change. Those values have important scope limits: GPU utilisation is not process-specific, temperature is a component reading, and battery drain is a coarse capacity delta.

The following fields remain unavailable from the current provider and are emitted as `value: null` with an `unavailable_reason`:

- `whole_device_power_watts`
- `energy_joules`
- `energy_watt_hours`

On platforms or hosts without a readable sensor, GPU, temperature, or battery events are also unavailable with that provider's reason. This is intentional. CPU percentage is not power, and unsupported power/energy/thermal values must not be represented as zero. `ResourceSampler` is the extension point for native iOS/Android measurements. `EnergyAccumulator` integrates available whole-device power samples with the trapezoidal rule and resets across unavailable intervals.

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
  "source": "sysinfo+linux-sysfs",
  "unavailable_reason": "no verified whole-device power meter; component power and battery-terminal power are not whole-device measurements"
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

The event sequence is run-local and monotonically assigned. The batch sequence is assigned by the local batcher. Retries use `(experiment_id, run_id, sequence_number)` as identity: identical content returns 202 without another append; different content returns 409. Do not rebatch the same event under another batch sequence; event identities across different batches are not deduplicated.

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

Malformed or inconsistent batches return `400`; unknown experiments return `404`; conflicting retries return `409`; bodies over 4 MiB return `413`. Storage survives restarts. The route does not automatically collect or verify a producer's measurements.

### Read batches

```http
GET /api/v1/experiments/:id/metrics
```

Returns:

```json
{
  "experiment_id": "exp-123",
  "batches": [/* stored batches */],
  "total": 1,
  "limit": 0,
  "offset": 0
}
```

This read endpoint supports exact `run_id`/`name` filters and optional `limit` (1–100)/`offset` pagination. Name filtering keeps matching events within batches; `total` counts matching batches. Without pagination the legacy response returns all matching batches and `limit: 0`; an offset requires an explicit limit. Ordering is run ID then batch sequence ascending. Unknown experiments return 404. It does not merge metrics into the generic experiment result; `/summary` reports separate observation statistics.

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

The versioned C ABI exposes an independent observer through `pheme_va_observer_new`, `pheme_va_observer_drain`, `pheme_va_observer_inspect`, and `pheme_va_observer_free`. It can drain while model inference runs and may outlive the engine. FFI retention is bounded to 512 events; overflow drops the oldest event and increments a cumulative dropped-event count. The event/batch wire schema remains unchanged.

The native host owns one drain consumer and distributes each original payload to displays, remote monitoring, and any future exporter. Retry uploaded batches with the same identity/content. Existing `MetricsBatcher::new()` consumers retain their previous retention behaviour.

`pheme_va_transcribe_v1` returns the same generated run ID used by its events; inspection reports the active run and admitted failures retain the run ID. An experiment ID is optional because the Go experiment route can supply it. FFI does not ingest measurements.

Collection is configured for future operations. Whisper-enabled builds also retain legacy `pheme_va_metrics_set_enabled` and `pheme_va_metrics_drain`; the legacy drain competes for the same queue and must not run beside another drain owner.

The [native host guide](../../pheme-va/hosts/ios/README.md) describes setup and the optional paired desktop monitor. The host does not yet upload batches to Go or collect device resource/energy measurements. Platform measurements remain host responsibilities and unavailable values must remain unavailable.
