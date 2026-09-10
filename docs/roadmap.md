# Implementation Roadmap

These are local planning references, not Linear issue IDs. Tickets are not created in Linear. Estimates are relative: S small, M medium, L large. T01 and T02 document the scope, service boundary, and initial API contract. T03 implements the validated service scaffold; real inference and workflow capabilities remain planned.

See [requirements](requirements.md), [architecture](architecture.md), and [benchmark methodology](benchmark-methodology.md). Build incrementally, retaining existing public contracts unless a ticket explicitly changes them.

## Rust backend migration — implemented foundation

The empty `pheme-va/` directory now contains the first portable backend slice. This does not claim that the complete incident workflow or mobile application is finished.

- `core`: audio/WAV validation, mono/16 kHz resampling, configurable energy gate, dictionary prompt construction, Whisper decoder settings, silence/prompt-echo guards, raw/processed transcript results, swappable STT and language-model cleanup traits, and a conservative rule-based incident extractor.
- `cli`: WAV transcription command and a small prewarmed-model terminal microphone recorder.
- `server`: development HTTP wrapper around the same core (`/health`, `/ready`, `/v1/transcribe`, `/v1/analyze`).
- `ffi`: C ABI bridge for eventual Swift/Kotlin hosts.
- Tests use generated WAV audio and local fakes. The ignored real-model test accepts externally supplied speech/model files; weights are not committed. The downloaded `large-v3-turbo` model has also been verified through the release CLI against a real speech sample.

The `voice-agent/` Python service remains the current contract scaffold until Rust contract parity, workflow coverage, and mobile/device validation justify a deliberate cutover. The next Rust work is to add a concrete llama.cpp-compatible cleanup adapter and run the pipeline on a physical iPhone.

## Milestone 1 — Real Local Incident Analysis

### T01 — Update project scope and architecture documentation (S)

Update AGENTS.md and README for the student-built incident workflow, removed Use Case 3, Whisper/llama.cpp baseline, and both web roles. Document requirements, architecture, evaluation, and delivery sequence; distinguish implemented behaviour and pending decisions.

**Acceptance:** documentation consistently reflects the KLASS direction and links to detailed plans. **Dependencies:** none. **Status:** covered by this documentation update.

### T02 — Define the Voice Agent service boundary and API contracts (M)

Name the dedicated Python service `voice-agent/`. Assign incident state, confirmation, storage/retrieval, and runtime adapters to it; retain the Go public API/client and benchmark responsibilities. Define the initial text-analysis request/response, unknown values, errors, timeouts/cancellation, and complete/incomplete/ambiguous examples. Outline future sessions and revision-bound confirmation without implementing them.

**Acceptance:** documentation has one owner for each state/data type and a precise initial analysis contract, while preserving the public endpoint. **Dependencies:** T01. **Status:** documented in [Voice Agent API](api/voice-agent.md) and [architecture](architecture.md). Future session schemas and finalization-completeness policy are explicitly deferred to T06.

### T03 — Scaffold the Voice Agent service with a validated text-analysis API (S)

Create `voice-agent/` with the [initial API contract](api/voice-agent.md), health/readiness endpoints, runtime/path/timeout configuration, tests, Python CI, and local setup instructions. Use a fake analyzer for contract tests; until T04 connects a runtime, return the documented unavailable error rather than presenting mock output as real analysis. Keep weights outside Git.

**Acceptance:** the service starts locally and validates requests; tests do not require downloaded models. **Dependencies:** T02. **Status:** implemented with health/readiness, configuration, tests, Ruff, and Python CI; real inference is T04b.

### T04a — Set up the local llama.cpp runtime (S)

Pin the upstream runtime revision, document installation outside the repository, and add a configurable launcher with executable/model path validation, port/context/thread/offload settings, version checks, and dry-run output. Defer final model selection; distinguish installation checks from model-backed inference checks.

**Acceptance:** pinned installation/version are verifiable, launcher tests need no model, and startup/shutdown instructions are reproducible. **Dependencies:** T03. **Status:** setup and launcher implemented; see [runtime instructions](../voice-agent/docs/llama-cpp.md). Model-backed verification is deferred.

### T04b — Connect Voice Agent to llama.cpp (M)

Use a provisional compatible model without committing to the final SLM. Add the Python runtime adapter, structured extraction prompts, output validation, real readiness checks, and timeout/unavailable-runtime handling. Record model, quantization, prompt/runtime revisions, and stage timing; run a model-backed smoke test.

**Acceptance:** Python returns real structured reports and handles invalid model output and runtime failures. **Dependencies:** T04a. This completes the original T04 inference milestone.

### T05 — Connect the Go incident analyzer to Voice Agent (S)

Implement VoiceAgentIncidentAnalyzer behind the existing interface, configure concrete wiring in main, propagate cancellation, and preserve simple HTTP errors. Add adapter tests and a cross-service smoke test.

**Acceptance:** the existing text endpoint performs real local analysis without changing its contract; mock mode remains usable. **Dependencies:** T04b.

## Milestone 2 — Complete Voice Reporting Workflow

### T06 — Implement conversation state and human confirmation (L)

Finalize the session/turn schemas and report-completeness policy outlined in T02. Implement the state machine and session/turn APIs in `voice-agent/`, initially using in-memory state. Add thin Go handlers/client methods that delegate to those APIs without duplicating transitions. Support clarification, correction, readback, cancellation, and confirmation of the current revision. Deduplicate retried turns.

**Acceptance:** text scenarios complete the workflow; stale or ambiguous confirmation cannot finalize a report, and corrections invalidate prior confirmation. **Dependencies:** T02, T05.

### T07 — Persist sessions and confirmed incident reports (M)

Add SQLite migrations and repositories inside `voice-agent/` for sessions, turns, and reports; Go must not access that database directly. Separate occurrence, recording, and confirmation timestamps. Make finalization atomic and idempotent.

**Acceptance:** reports survive restart, sessions resume consistently, and retries cannot create duplicates. **Dependencies:** T06.

### T08 — Add Whisper transcription and audio turns (M)

Implement a replaceable transcription adapter inside `voice-agent/`, provisionally whisper.cpp. Forward audio through the Go client to the Voice Agent. Accept/normalize supported audio formats; enforce limits and handle silence, cancellation, and invalid input. Record runtime configuration and transcription duration.

**Acceptance:** audio enters the same conversation workflow as text and failures are recoverable. **Dependencies:** T03, T06.

### T09 — Build the web voice console and spoken responses (M)

Add push-to-talk, speech output through an adapter, listening/processing/speaking status, and development views of transcript/draft/state. Route spoken corrections and confirmation through the application workflow. Document speech-synthesis execution location.

**Acceptance:** a user reports, corrects, and confirms an incident through spoken turns; offline/local claims match the implementation. **Dependencies:** T07, T08.

### T10 — Implement incident retrieval and spoken summaries (M)

Add Voice Agent listing/detail APIs, thin Go delegation, and validated time/count filters with parameterized queries in Voice Agent repositories. Interpret spoken requests and summarize only retrieved records, retaining IDs for verification. Cover empty and partial result sets.

**Acceptance:** “last five reports in the last hour” returns the correct records and grounded spoken answer. **Dependencies:** T07, T09.

## Milestone 3 — Benchmarking and Research Dashboard

### T11 — Create the incident evaluation dataset and scoring rules (M)

Collect consented local-speech examples covering terminology, noise, missing facts, corrections, confirmation, and retrieval. Annotate expected outputs, separate development/held-out data, and define scoring. Keep larger media external with versioned manifests.

**Acceptance:** the same documented dataset/scoring procedure can evaluate multiple configurations without tuning on held-out data. **Dependencies:** T02. Start early alongside inference work.

### T12 — Replace the mock benchmark runner with an incident runner (L)

Run real versioned scenarios against the Voice Agent service API through the shared Go client; include separate public-API end-to-end checks. Collect stage/resource metrics, outcomes, and full runtime configuration. Distinguish execution errors and output-quality failure. Explicitly remove licence-plate monitoring from supported benchmark requests and update affected tests/API docs; interview development remains deferred.

**Acceptance:** experiments produce measured workflow results, identify unavailable metrics, and no longer accept the removed use case. **Dependencies:** T10, T11. Basic timing instrumentation starts in T04b/T08.

### T13 — Persist experiments and expose dashboard query APIs (M)

Add a Go-owned SQLite experiment/result repository, paginated history and filters. Keep experiment storage separate from Voice Agent incident/session storage. Preserve create/get contracts and define interrupted-run handling after restart without assuming the in-memory queue is durable.

**Acceptance:** historical runs survive restart and can be queried; interrupted runs have a defined visible outcome. **Dependencies:** T07, T12.

### T14 — Build the benchmark dashboard (L)

Add experiment configuration/creation, history, progress polling, and result details. Show quality, latency, memory, and available device metrics with clear loading/empty/error/unavailable states. Follow existing frontend conventions.

**Acceptance:** researchers can create a run and inspect measured results without curl. **Dependencies:** T13.

### T15 — Add experiment comparison and export (M)

Compare selected models, quantization, runtimes, and devices. Display configuration differences and flag different datasets or measurement methods. Add useful charts and export with reproducibility metadata.

**Acceptance:** researchers can compare compatible runs and export results with configuration and measurement context. **Dependencies:** T14.

## Milestone 4 — Target-Device Evaluation

### T16 — Confirm target hardware and deployment constraints (S)

Obtain device/OS/RAM/accelerator/storage/battery details, local/offline requirements, instrumentation access, thermal conditions, and the operating workload. Clarify the illustrative 8–10-hour target and robotics scope.

**Acceptance:** device constraints and evaluation expectations are recorded; unresolved items are explicit. **Dependencies:** KLASS specifications. Request alongside implementation; do not block the prototype.

### T17 — Deploy the baseline workflow to the selected device (L)

After selection, split into device-specific tasks: service packaging for Jetson or native inference/workflow integration for iOS. Reuse schemas, prompts, and conformance scenarios. Validate the whole speech/report/retrieval loop and document setup.

**Acceptance:** the baseline runs on the actual target at the agreed execution location; remote inference is not presented as on-device execution. **Dependencies:** T10, T16.

### T18 — Measure device efficiency and evaluate optimizations (L)

Implement device measurement collection and compare a bounded set of model/runtime configurations. Measure idle/active power, energy per task, temperatures, sustained performance, and agreed endurance. Evaluate local-speech improvements on held-out examples and present quality/energy trade-offs in the dashboard.

**Acceptance:** repeated measurements, configurations, and workload definitions support a reproducible deployment recommendation; whole-device and component measurements are distinguished. **Dependencies:** T11, T12, T15, T17.

## Recommended Starting Sequence and Checks

With T01/T02 documented and T03 scaffolded, continue with T04b–T05, start T11 using the contract examples, and request T16 information in parallel with development. Then complete the spoken workflow before expanding dashboard comparisons and device optimization. Do not assign calendar deadlines until team capacity and device availability are known.

For implementation tickets, use meaningful Go/Python unit tests at integration/state boundaries and shared cross-service scenarios. Run gofmt, go vet, and Go tests for backend changes, and lint/build for frontend changes. Ordinary CI should use fakes/small fixtures; model and device evaluations run separately with recorded configurations. For documentation-only updates, verify diff whitespace, links, current-code claims, and consistency; no runtime tests are required.
