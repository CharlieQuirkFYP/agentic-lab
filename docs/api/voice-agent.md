# Pheme VA Host and Voice-Agent API Contract

This document describes the current Rust Pheme VA development host and the planned stateful workflow boundary. The former Python `voice-agent/` runtime was removed and is not an active service.

## Current status

Implemented in `pheme-va/`:

- a portable Rust audio/transcription core with replaceable `Transcriber` and cleanup/model seams;
- optional in-process `whispercpp` and Zipformer CLI adapters;
- an Axum development server with stateless transcription, deterministic incident extraction, health/readiness, and an in-memory metrics-batch drain;
- a direct-path Whisper C ABI for future native hosts; and
- a local TUI that provides developer run history and metrics, not operational incident sessions.

The Go API remains a separate public API. It is currently wired to `MockIncidentAnalyzer`, and no Go-to-Pheme client is configured. Stateful sessions, clarification, revision-bound confirmation, incident persistence/retrieval, speech synthesis, and action execution remain planned.

## Service identity and ownership

**Pheme VA** is the canonical Rust implementation under `pheme-va/`. The current core owns audio normalization, transcription adapter boundaries, transcript guards, cleanup boundaries, and the conservative rule-based incident extractor. The future Pheme workflow will own conversation/session state, drafts, clarification, corrections, confirmation, incident storage/retrieval, and response generation.

The current development server is not yet the complete workflow service. It loads a Whisper model at startup and exposes stateless HTTP operations. The CLI can additionally select the optional Zipformer adapter by manifest ID. The FFI is direct-path Whisper based and does not implement manifest selection or model reload.

Go owns its public HTTP contract, benchmark orchestration, experiment configuration/results, and separate metrics storage. Web owns the future voice interaction/playback client and benchmark dashboard. Services must access their own repositories only once persistence is added.

## Current Rust HTTP API

The development server requires a model path through `--model` or `PHEME_VA_WHISPER_MODEL` and listens on `127.0.0.1:8000` by default. Build it with the `whisper` feature:

```bash
cargo run --release -p server --features whisper -- \
  --model models/whisper/ggml-large-v3-turbo.bin
```

Current routes:

| Method | Route                 | Purpose                                   |
| ------ | --------------------- | ----------------------------------------- |
| `GET`  | `/health`             | Liveness                                  |
| `GET`  | `/ready`              | In-process Whisper readiness              |
| `POST` | `/v1/transcribe`      | One complete WAV request                  |
| `POST` | `/v1/analyze`         | Stateless transcript-to-report extraction |
| `GET`  | `/v1/metrics/batches` | Drain buffered metric batches             |

These routes are separate from the Go routes under `/api/v1/...`.

### `GET /health`

The liveness response is always unwrapped JSON when the process is serving:

```json
{
  "status": "ok"
}
```

It does not inspect the model.

### `GET /ready`

A successfully loaded Whisper engine reports:

```json
{
  "status": "ready"
}
```

If the engine is unavailable, the custom response is `503 Service Unavailable`:

```json
{
  "error": {
    "code": "runtime_unavailable",
    "message": "Analysis runtime is unavailable."
  }
}
```

Model-load failures occur before the listener starts, so they are startup errors rather than `/ready` responses.

### `POST /v1/transcribe`

The body is one complete WAV file. JSON and multipart uploads are not accepted.

Accepted content types are:

- `audio/wav`
- `audio/x-wav`

Media-type parameters are tolerated. The body must be a WAV container; bare PCM is not accepted. The core supports 8-, 16-, 24-, and 32-bit integer WAV input and 32-bit floating-point WAV input, with arbitrary supported sample rates and channel counts. It downmixes to mono and resamples to 16 kHz before invoking the selected model. `--max-seconds` applies to the original WAV duration.

Example:

```bash
curl -i -X POST http://127.0.0.1:8000/v1/transcribe \
  -H 'Content-Type: audio/wav' \
  --data-binary @recording.wav
```

A successful response is an unwrapped serialized `TranscriptionResult`. The exact enum values use Rust variant casing because the current types do not apply a serde rename rule:

```json
{
  "status": "Speech",
  "raw_text": "vehicle collided near the west entrance",
  "text": "Vehicle collided near the west entrance",
  "language": "en",
  "model_id": "whisper-large-v3-turbo",
  "model_family": "whisper",
  "model_revision": null,
  "segments": [
    {
      "start_ms": 0,
      "end_ms": 1840,
      "text": "vehicle collided near the west entrance",
      "no_speech_probability": 0.02
    }
  ],
  "gate": {
    "decision": "SpeechDetected",
    "peak_rms": 0.12,
    "peak_amplitude": 0.74,
    "window_count": 92,
    "speech_window_count": 61,
    "max_consecutive_speech_windows": 37
  },
  "dictionary_prompt": null,
  "stt_backend": "whisper-large-v3-turbo",
  "cleanup_backend": "rule-based-formatter",
  "cleanup_status": "Applied",
  "recovery_attempted": false,
  "processing_time_ms": 742
}
```

The current server's direct Whisper wiring reports the supplied model path as `model_id`, `model_family: "whisper"`, and no revision. It does not load `models/manifest.toml`; the CLI's manifest-based loader provides stable model IDs and revision metadata for supported adapters.

Important fields:

| Field                                          | Current behavior                                                                                               |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `status`                                       | `Speech`, `NoSpeech`, `InsufficientSpeech`, `DictionaryPromptEcho`, `KnownSilenceMarker`, or `EmptyTranscript` |
| `raw_text`                                     | Normalized recognizer output before cleanup                                                                    |
| `text`                                         | Final returned transcript; rule-based cleanup only normalizes whitespace/capitalization                        |
| `language`                                     | Detected or configured language, if available                                                                  |
| `model_id` / `model_family` / `model_revision` | Selected model metadata                                                                                        |
| `segments`                                     | Recognizer segments with millisecond timestamps when the adapter supplies them                                 |
| `gate`                                         | Speech-gate decision and energy statistics                                                                     |
| `dictionary_prompt`                            | Prompt generated from configured dictionary terms, or `null`                                                   |
| `stt_backend`                                  | Adapter identifier                                                                                             |
| `cleanup_backend` / `cleanup_status`           | Cleanup implementation and outcome                                                                             |
| `recovery_attempted`                           | Whether the dictionary-prompt recovery attempt ran                                                             |
| `processing_time_ms`                           | End-to-end core processing time                                                                                |

A silent or insufficient-speech WAV is a valid processed result and returns `200`, commonly with empty text and `gate.decision` of `Silence` or `InsufficientSpeech`. It is not an HTTP failure.

Transcription error mapping:

| Condition                                                                                 | Status | Code                     |
| ----------------------------------------------------------------------------------------- | -----: | ------------------------ |
| Missing or unsupported content type                                                       |  `415` | `unsupported_media_type` |
| Empty request body                                                                        |  `400` | `invalid_request`        |
| WAV exceeds `--max-seconds`                                                               |  `400` | `audio_too_long`         |
| Runtime is unavailable                                                                    |  `503` | `runtime_unavailable`    |
| Invalid WAV, unsupported WAV data, backend failure, poisoned engine lock, or task failure |  `500` | `transcription_failed`   |

Malformed or unsupported WAV data currently maps to the generic `500` response rather than a structured `400` response. The server buffers a complete body before parsing and processing it; there is no WebSocket, live microphone, partial-transcript, or incremental audio endpoint.

### `POST /v1/analyze`

This route runs `RuleBasedIncidentAnalyzer` against a transcript. It does not call an LLM, create a session, save an incident, ask clarification questions, confirm a report, or execute `recommended_action`. There is no combined audio-to-analysis route: call `/v1/transcribe` first and send its text here if needed.

Request:

```http
POST /v1/analyze
Content-Type: application/json
```

```json
{
  "transcript": "A vehicle collided with a barrier near the west entrance. Severity is low. Please notify the site supervisor."
}
```

Request rules:

- `transcript` is required and must be a string;
- surrounding whitespace is removed before validation;
- the trimmed transcript must be non-empty and at most 16,000 Unicode characters; and
- unknown JSON fields are rejected.

The successful response has exactly five string fields:

```json
{
  "incident_type": "collision",
  "location": "the west entrance",
  "severity": "low",
  "summary": "A vehicle collided with a barrier near the west entrance. Severity is low. Please notify the site supervisor.",
  "recommended_action": "Please notify the site supervisor"
}
```

Current extraction is deliberately simple:

| Field                | Current behavior                                                                      |
| -------------------- | ------------------------------------------------------------------------------------- |
| `incident_type`      | Keyword category: collision, smoke report, fire report, injury report, or `unknown`   |
| `location`           | Text after the earliest `near`, `at`, `by`, or `in` marker, or `unknown`              |
| `severity`           | Explicit `low`, `medium`, or `high`, or `unknown`                                     |
| `summary`            | The trimmed transcript                                                                |
| `recommended_action` | Text after the earliest `please`, `notify`, `call`, or `contact` marker, or `unknown` |

The analyzer copies explicit text and does not classify risk or execute actions. It does not yet preserve all ambiguity semantically; for example, an uncertain statement containing “collision near the east or west gate” is still handled by the simple keyword/marker rules. Preserving uncertainty and asking clarification questions belongs to the future workflow.

Analysis errors:

| Condition                                           | Status | Response                                 |
| --------------------------------------------------- | -----: | ---------------------------------------- |
| Blank or overlong transcript                        |  `400` | Custom `invalid_request` envelope        |
| Analyzer returns an invalid report                  |  `502` | Custom `invalid_model_response` envelope |
| Malformed JSON                                      |  `400` | Axum default JSON rejection              |
| Missing/non-string/null transcript or unknown field |  `422` | Axum default JSON rejection              |
| Missing/non-JSON content type                       |  `415` | Axum default JSON rejection              |

Custom validation errors use:

```json
{
  "error": {
    "code": "invalid_request",
    "message": "A non-empty transcript of at most 16000 characters is required."
  }
}
```

Axum extractor rejection bodies currently use their default plain-text responses rather than this custom envelope.

## Metrics correlation and drain

The Rust transcription and analysis routes accept optional headers for metric correlation:

| Header              | Behavior                                                              |
| ------------------- | --------------------------------------------------------------------- |
| `X-Run-ID`          | Trimmed non-empty value becomes the metrics run ID                    |
| `X-Experiment-ID`   | Optional experiment correlation ID                                    |
| `X-Incident-Active` | Accepts `true`, `1`, `yes`, `false`, `0`, or `no`, case-insensitively |

If `X-Run-ID` is absent, the server generates a process-local ID such as `run_<epoch_milliseconds>_<counter>`. These headers affect metrics only and are not authentication credentials.

Metric collection is disabled by default. Enable application events with `--metrics-enabled` or `PHEME_VA_METRICS_ENABLED=true`; resource snapshots additionally require `--resource-sampling` or `PHEME_VA_RESOURCE_SAMPLING=true`. `/v1/analyze` enables incident-only metrics by default unless a valid `X-Incident-Active: false` header overrides them. `/v1/transcribe` uses the startup `--incident-metrics` setting unless overridden.

`GET /v1/metrics/batches` drains all currently buffered batches as a JSON array and removes them from the in-memory batcher. It has no filters, acknowledgements, retry queue, or persistence. Events can be forwarded by an external exporter to the Go route `POST /api/v1/experiments/:id/metrics`; the Rust server does not know the Go URL or forward automatically. See [the metrics contract](metrics.md) for event fields and resource availability.

## Current Go public API

The Go endpoint remains unchanged while Pheme integration is planned:

```http
POST /api/v1/incidents/analyze
Content-Type: application/json
```

The request contains `transcript`. Its success response contains the five fields `incident_type`, `location`, `severity`, `summary`, and `recommended_action`. The current `MockIncidentAnalyzer` returns `unknown` for type/location/severity, echoes the transcript as `summary`, and returns `Pending AI analysis` as `recommended_action`. Binding failures return `400` with a simple error; analyzer failures return a generic `500` response. The public handler does not forward Rust internal errors because no Pheme client is wired yet.

## Future session and retrieval outline

The routes below are planned Pheme-owned Rust routes, not current endpoints. The Go routes would be thin public delegations once implemented.

| Planned Rust route              | Planned Go route                    | Purpose                                    |
| ------------------------------- | ----------------------------------- | ------------------------------------------ |
| `POST /v1/sessions`             | `POST /api/v1/sessions`             | Allocate a service-owned session           |
| `GET /v1/sessions/:id`          | `GET /api/v1/sessions/:id`          | Read state and current draft               |
| `POST /v1/sessions/:id/turns`   | `POST /api/v1/sessions/:id/turns`   | Submit text or later audio with a retry ID |
| `POST /v1/sessions/:id/confirm` | `POST /api/v1/sessions/:id/confirm` | Confirm a specific draft revision          |
| `POST /v1/sessions/:id/cancel`  | `POST /api/v1/sessions/:id/cancel`  | Abandon a draft                            |
| `GET /v1/incidents`             | `GET /api/v1/incidents`             | Query finalized reports                    |
| `GET /v1/incidents/:id`         | `GET /api/v1/incidents/:id`         | Retrieve one report                        |

The future state machine should use `collecting`, `awaiting_confirmation`, `saved`, and `cancelled` states. A correction increments the revision and invalidates prior confirmation. Ambiguous consent remains unconfirmed. Retry IDs replay the prior result for an identical request; conflicting reuse and stale revisions return explicit conflicts. Finalization must be transactional and idempotent. Exact schemas, completeness policy, concurrency behavior, audio envelope, retention, and session expiry remain implementation decisions for the workflow milestone.

Future retrieval should use validated time/count filters and parameterized queries inside Pheme-owned repositories. The proposed baseline for recorded-time queries is UTC, inclusive `from`, exclusive `to`, newest-first ordering with a stable ID tie-breaker, and a limit of five for the example query. Summaries must use only records returned by the repository and retain their IDs for grounding checks.

## Boundaries and next steps

Do not add a Python runtime, database server, distributed queue, or unrestricted action executor to satisfy this contract. The next implementation steps are a Go Pheme client, an opt-in validated language-model adapter if needed, the Pheme-owned session state machine, SQLite persistence, audio turns, retrieval, and then client/dashboard integration. The current stateless routes remain compatibility/development operations while that work proceeds.
