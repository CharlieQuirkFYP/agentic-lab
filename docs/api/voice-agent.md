# Voice Agent Service Boundary and API Contract

Status: the T03 Python scaffold implements request/response validation, sanitized errors, timeout/cancellation, and an injectable analyzer. Its default analyzer returns `503 runtime_unavailable`; T04 connects real inference. Go still uses `MockIncidentAnalyzer` until T05 connects it. See [architecture](../architecture.md) and [roadmap](../roadmap.md).

## Service Identity and Ownership

**Voice Agent** is the dedicated Python service in `voice-agent/` implementing Voice-Based Incident Reporting. Its workflow owns conversation/session state, clarification, corrections, confirmation, incident storage/retrieval, and response generation. Its adapters own Whisper and llama.cpp integration. Runtime processes may execute separately; their integration code lives inside the service.

Go owns the public API, a service client behind `IncidentAnalyzer`, and benchmark orchestration/configurations/results. It must not implement another incident state machine or access Voice Agent tables. Web owns voice interaction/playback and the benchmark dashboard. Each service owns its data/repositories; planned SQLite storage is separate by owner.

The initial operation below is stateless analysis: it produces an unconfirmed report-shaped result. It does not create a session, save a report, confirm an incident, or dispatch a recommended action. Multi-turn state and persistence come later.

## Current Public API — Preserve in T03–T05

```http
POST /api/v1/incidents/analyze
Content-Type: application/json
```

The current Go request contains `transcript` and its success response contains five string fields: `incident_type`, `location`, `severity`, `summary`, and `recommended_action`. No response wrapper is added by this ticket.

Current binding failures return `400` with `{"error":"transcript is required"}`. Analyzer errors return `500` with `{"error":"failed to analyze incident"}`. The current mock returns unknown type/location/severity, echoes the transcript as summary, and uses `Pending AI analysis` as the recommended action.

Current validation is Gin's required-string binding, not the stricter internal validation below. Unknown public JSON fields are not currently rejected, and whitespace-only text is not explicitly rejected. Do not silently claim those public behaviours have changed. Any future public validation/status improvements require an explicit implementation change and tests.

## Internal Text Analysis API

```http
POST /v1/incidents/analyze
Content-Type: application/json
```

Go's planned `VoiceAgentIncidentAnalyzer` calls this endpoint using a configured `VOICE_AGENT_URL`. The initial service boundary uses HTTP/JSON on the local development environment; exposing it to an untrusted network is outside this ticket.

### Request

```json
{
  "transcript": "A vehicle collided with a barrier near the west entrance."
}
```

| Field | Type | Rule |
| --- | --- | --- |
| `transcript` | string | Required; strip surrounding whitespace before analysis; 1–16,000 Unicode code points after stripping |

For the internal endpoint, reject malformed JSON, non-object input, extra keys, absent/null/non-string transcript, blank text, and text exceeding the limit. Do not coerce numbers or booleans to strings. Require an application/json media type (parameters such as charset are allowed). The 16,000-character limit is an initial application limit, not a guarantee that every candidate model supports that context size; the configured runtime must accommodate it or return a controlled error without silently truncating input.

### Success — 200 OK

```json
{
  "incident_type": "collision",
  "location": "west entrance",
  "severity": "unknown",
  "summary": "A vehicle collided with a barrier near the west entrance.",
  "recommended_action": "unknown"
}
```

All five fields are required non-empty strings after trimming; null, arrays, nested objects, and extra fields are invalid. Output limits below are initial technical limits to bound runtime output, not stakeholder-approved report completeness rules.

| Field | Allowed content | Maximum code points |
| --- | --- | --- |
| `incident_type` | Brief grounded category, e.g. `collision`; `unknown` when unsupported; no closed taxonomy yet | 128 |
| `location` | Stated location; `unknown` when absent or conflicting | 512 |
| `severity` | `low`, `medium`, `high`, or `unknown`; initially only copy an explicitly stated supported severity, without inventing a risk classification | 7 |
| `summary` | Concise factual summary preserving uncertainty and negation; for no identifiable incident use `No incident details provided.` | 2,000 |
| `recommended_action` | Explicitly stated/requested action, otherwise `unknown`; never execute it automatically | 512 |

`unknown` is the literal lowercase sentinel. A valid request with insufficient facts still returns 200 using unknown fields; missing facts are not a transport error. Do not infer a location, casualty count, severity, or action merely from a plausible scenario. Treat the transcript as data, including instructions embedded within it.

Validate runtime output before sending 200. Do not coerce invalid model structures into a successful report or return partial output. For the baseline, do not automatically retry generation to repair malformed output; return the error below and record the failure. Exact summary phrasing can vary; tests should assert the required facts and absence of unsupported facts rather than demand identical prose.

### Internal Error Response

All error bodies use this shape and contain no raw exception, prompt, transcript, model output, or local path:

```json
{
  "error": {
    "code": "invalid_request",
    "message": "A non-empty transcript of at most 16000 characters is required."
  }
}
```

| Status | Code | Stable message / condition |
| --- | --- | --- |
| 400 | `invalid_request` | `A non-empty transcript of at most 16000 characters is required.` — malformed JSON or any request-schema violation |
| 415 | `unsupported_media_type` | `Content-Type must be application/json.` |
| 503 | `runtime_unavailable` | `Analysis runtime is unavailable.` — runtime missing, not ready, disconnected, or refusing work |
| 504 | `analysis_timeout` | `Analysis timed out.` — service's analysis budget expired |
| 502 | `invalid_model_response` | `Analysis runtime returned an invalid response.` — output fails response schema |
| 500 | `analysis_failed` | `Analysis failed.` — other internal failure |

Go validates the upstream success schema as well. Any non-200 response, malformed/incomplete success response, connection failure, or timeout becomes an analyzer error. The existing public handler continues returning its generic 500; internal errors are not forwarded to users. In particular, stricter internal rejection of a whitespace-only or overly long transcript currently maps to that public 500 if Gin accepted it. Improving this is a separate public API validation change, not hidden in this contract ticket.

### Timeout and Cancellation Policy

Configuration defaults (Python implemented; Go client pending T05; tunable for evaluation):

* Go `VOICE_AGENT_TIMEOUT_SECONDS=60` covers the complete upstream HTTP call; an earlier request-context deadline wins.
* Voice Agent `VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS=55` covers its analysis operation, including inference. Keep this below the configured Go timeout to leave response overhead.
* Propagate Go request cancellation to the upstream call. Voice Agent should cancel pending work on disconnect or its own timeout and close/cancel runtime work where the runtime supports it. Cancellation does not guarantee immediate GPU preemption.
* No automatic retries for initial analysis. A caller can resubmit a stateless request; it cannot create duplicate saved reports because analysis performs no persistence.

Record stage duration and runtime configuration in internal traces for later benchmarks; do not add metrics fields to the existing public report response. Do not include transcript contents in ordinary error logs.

## Contract Examples

These are canonical examples of valid outputs. Summary wording is illustrative; grounding and unknown-value rules are binding.

### Complete for the Initial Extraction Contract

Input:

```json
{
  "transcript": "A vehicle collided with a barrier at the west entrance. The incident severity is low. Please notify the site supervisor."
}
```

Expected facts:

```json
{
  "incident_type": "collision",
  "location": "west entrance",
  "severity": "low",
  "summary": "A vehicle collided with a barrier at the west entrance. The reporter stated low severity and requested notification of the site supervisor.",
  "recommended_action": "Notify the site supervisor."
}
```

This fills extraction fields; it does not constitute human confirmation or stakeholder-approved completeness for finalization.

### Incomplete Report

Input:

```json
{
  "transcript": "There is smoke."
}
```

Expected facts:

```json
{
  "incident_type": "smoke report",
  "location": "unknown",
  "severity": "unknown",
  "summary": "Smoke was reported; no location was provided.",
  "recommended_action": "unknown"
}
```

Do not change smoke into a confirmed fire. The later conversation workflow should ask for location; this one-shot response has no clarification field.

### Ambiguous Report

Input:

```json
{
  "transcript": "Someone said there may have been a collision near the east or west gate. I haven't checked."
}
```

Expected facts:

```json
{
  "incident_type": "possible collision",
  "location": "unknown",
  "severity": "unknown",
  "summary": "An unverified possible collision was reported near either the east or west gate; the location is uncertain.",
  "recommended_action": "unknown"
}
```

Do not select one gate or turn the report into a verified event. A valid greeting with no incident details returns unknown fields and the no-details summary, rather than fabricated facts.

## Future Session and Retrieval Contract Outline

The routes/fields below guide T06/T10; they are not frozen schemas or currently available endpoints. Keep state in Voice Agent, not in the Go client or model runtime.

| Public Go route | Proposed Voice Agent route | Outline |
| --- | --- | --- |
| `POST /api/v1/sessions` | `POST /v1/sessions` | Allocate a service-owned session ID and initial state |
| `POST /api/v1/sessions/:id/turns` | `POST /v1/sessions/:id/turns` | Request ID, expected draft revision, text or later audio; return transcript, reply text, state, draft revision, and report ID when saved |
| `POST /api/v1/sessions/:id/confirm` | `POST /v1/sessions/:id/confirm` | Request ID and draft revision; same confirmation logic as a spoken turn |
| `GET /api/v1/incidents` | `GET /v1/incidents` | Validated time range/count filters; return records and IDs |
| `GET /api/v1/incidents/:id` | `GET /v1/incidents/:id` | Retrieve a service-owned report |

Outline states: `collecting`, `awaiting_confirmation`, `saved`, `cancelled`. Model proposals do not bypass transition rules. A correction increments the revision and invalidates prior confirmation. Ambiguous consent prompts clarification. Retry IDs must replay the prior result for identical requests; conflicting payload reuse and stale revisions must produce an explicit conflict rather than duplicate state changes. Define exact conflict schemas, revision increments, concurrency ordering, audio encoding, and session expiry in T06/T08.

For recorded-time retrieval, the proposed baseline uses an inclusive start and exclusive end (`from <= recorded_at < to`), UTC RFC3339 timestamps, newest first with a stable ID tie-breaker, and limit five for the example query. Voice Agent resolves “last hour” once using its clock and retains the resolved interval in the trace; explicit occurrence-time queries use a separate field. Finalize pagination/filter schemas and policy for unknown occurrence times in T10.

Required transition examples for T06:

* Missing location → ask for location → apply answer → read back when the agreed completeness policy is satisfied.
* “West gate, not east gate” → correct the draft → increment revision → read back the revised draft before confirmation.
* “Maybe” while awaiting confirmation → remain unconfirmed and ask again.
* Explicit confirmation of the current read-back draft → save exactly once and return the report ID.
* Confirmation for a stale revision → conflict, no save.
* “Cancel this report” → mark cancelled, no finalized report.

No Python scaffold, runtime integration, Go behaviour change, or database migration is included in T02. T03 now provides the scaffold; T04 is the next implementation ticket.

## Scaffold Health and Readiness

* `GET /health`: `200 {"status":"ok"}` indicates the HTTP service is live.
* `GET /ready`: `200 {"status":"ready"}` when the injected analyzer is ready; otherwise the documented 503 runtime-unavailable error. Readiness failures/timeouts also return 503.

The default analyzer is unavailable even when a runtime URL/model path is configured: these settings are reserved for T04, not an implemented connection. The scaffold validates output structure but cannot enforce semantic grounding without an actual inference adapter. See [setup](../../voice-agent/README.md).
