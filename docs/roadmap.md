# Implementation Roadmap

These are local planning references, not Linear issue IDs. Estimates are relative: S small, M medium, L large. The former Python Voice Agent plan no longer describes the implementation path: `pheme-va/` is the canonical Rust workspace, and the Python runtime was removed.

See [requirements](requirements.md), [architecture](architecture.md), and [benchmark methodology](benchmark-methodology.md). Build incrementally and retain existing public contracts unless an implementation ticket explicitly changes them.

## Current implementation status

The following foundation is implemented on this branch:

- `pheme-va/crates/core`: WAV/PCM validation, 8/16/24/32-bit integer and 32-bit float WAV decoding, downmixing, windowed-sinc resampling to mono 16 kHz, configurable energy gating, dictionary prompts, decoder options, transcript guards, cleanup traits, model-neutral transcription, and conservative rule-based incident extraction.
- `pheme-va/crates/metrics`: versioned scalar events, per-run contexts, synchronous pub/sub, stage timers, resource snapshots, explicit unavailable values, desktop sampling, Linux GPU/temperature/battery extensions, and batch formation.
- `pheme-va/crates/models/whispercpp`: optional prewarmed in-process whisper.cpp adapter with model metadata and model timing support.
- `pheme-va/crates/models/zipformer`: optional LiteRT Zipformer CTC adapter for the manifest's small/medium/large variants.
- `pheme-va/crates/cli`: manifest-driven `transcribe` and `tui` commands; TUI onboarding, WAV browsing, live microphone capture, worker-owned model loading/switching, metric graphs/details, structured logs, allowlisted model downloads, adapter cache/build/restart, and bounded JSON run history.
- `pheme-va/crates/server`: development Axum host with `/health`, `/ready`, `/v1/transcribe`, `/v1/analyze`, and `/v1/metrics/batches`.
- `pheme-va/crates/ffi`: direct-path Whisper C ABI with optional metrics-batch draining for future native hosts.
- `api/`: Gin public API, asynchronous in-memory benchmark lifecycle, and separate in-memory metric-batch ingestion.
- `web/`: React/Vite toolchain scaffold and placeholder page.

This foundation does not include the stateful incident workflow, confirmed-report persistence, retrieval, speech synthesis, Go-to-Pheme integration, a real benchmark runner, or a completed web console/dashboard.

## Completed documentation and foundation milestones

### T01 — Confirm project scope and research direction (S) — complete

Document the student-built speech incident workflow, removed vision/licence-plate use case, deferred interview work, local-deployment research objective, pending device decisions, and both web roles.

### T02 — Define the Pheme VA boundary and stateless host contract (M) — complete

Document Pheme VA as the Rust owner of the portable audio/transcription foundation and future workflow. Record the current Rust HTTP routes, request/response shapes, errors, metrics correlation, and the future session/retrieval outline without presenting those future routes as available.

### T03 — Build the portable Rust audio and transcription foundation (M) — complete

Implement the reusable core, optional whisper.cpp and Zipformer adapter boundaries, model manifest, development server, CLI, FFI bridge, tests, and reproducible ignored model setup. Keep model weights and recordings outside Git.

### T04 — Build the local evaluation TUI and metrics foundation (L) — complete baseline

Implement the Ratatui developer console, manifest model picker, WAV/microphone paths, asynchronous worker, live model switching, telemetry tabs, metric details, historical run reports, bounded logs, native diagnostics on Unix, allowlisted artifact downloads, adapter preparation/cache restart, persistent local run history, resource sampling, and retry/error states.

The TUI is a development/model-evaluation console. It does not implement the production incident workflow, partial streaming transcription, or verified whole-device power measurement. The Rust server and FFI remain direct-path Whisper hosts even though the CLI can select Zipformer.

## Milestone 1 — Connect the Go API to Pheme VA

### T05 — Connect the Go incident analyzer to Pheme VA (S)

Implement a Pheme-backed analyzer/client behind the existing Go `IncidentAnalyzer` interface, configure concrete wiring in `api/cmd/server/main.go`, propagate request cancellation/timeouts, validate the upstream response, and preserve simple public HTTP errors. Retain `MockIncidentAnalyzer` for isolated tests and local mock mode.

**Acceptance:** the existing Go text endpoint can perform the current stateless Rust analysis when configured, without changing its public request/response contract. **Dependencies:** T02, T03.

### T05b — Add a replaceable structured language-model adapter (M)

Add a concrete local language-model adapter only after the runtime, model, prompt, output validation, timeout, and semantic-grounding contract are selected. Keep deterministic rule-based analysis as the offline default and record model/runtime metadata. This is not currently implemented; no llama.cpp runtime or Python adapter is assumed.

**Acceptance:** model-backed extraction is opt-in, validated, cancellable within the documented budget, and tested with local fakes plus a separately supplied smoke-test model. **Dependencies:** T05 and a runtime/model decision.

## Milestone 2 — Complete the voice reporting workflow

### T06 — Implement conversation state and human confirmation (L)

Add Pheme-owned session/turn APIs and an explicit state machine, initially using in-memory state if useful. Support clarification, correction, readback, cancellation, confirmation of the current draft revision, and idempotent retry handling. Add thin Go handlers/client methods only when the Pheme routes exist; do not duplicate transitions in Go.

**Acceptance:** text scenarios complete the workflow; stale or ambiguous confirmation cannot finalize a report, and corrections invalidate prior confirmation. **Dependencies:** T02, T05.

### T07 — Persist sessions and confirmed incident reports (M)

Add Pheme-owned SQLite migrations and repositories for sessions, turns, draft revisions, and reports. Keep Go away from Pheme tables. Separate occurrence, recording, and confirmation timestamps; make finalization atomic and idempotent.

**Acceptance:** reports survive restart, sessions resume consistently, and retries cannot create duplicate reports. **Dependencies:** T06.

### T08 — Add audio turns to the stateful workflow (M)

Route WAV/audio turns through Pheme's existing normalization and transcription path into the same session workflow as text. Enforce body/duration limits, handle silence, cancellation, invalid audio, and model errors, and record transcription/runtime metadata. The stateless Rust `/v1/transcribe` endpoint and CLI already provide the underlying audio path; this ticket connects it to sessions rather than creating a second recognizer.

**Acceptance:** audio enters the same clarification/correction/confirmation workflow as text and failures are recoverable. **Dependencies:** T03, T06.

### T09 — Build the web voice console and spoken responses (M)

Add push-to-talk, WAV/audio handling, speech output through a replaceable client/platform adapter, listening/processing/speaking status, and development views of transcript/draft/state. Route spoken corrections and confirmation through the Pheme-owned workflow. Document where speech synthesis executes.

**Acceptance:** a user can report, correct, and confirm an incident through spoken turns; offline/local claims match the implementation. **Dependencies:** T07, T08.

### T10 — Implement incident retrieval and spoken summaries (M)

Add Pheme listing/detail routes, thin Go delegation, validated time/count filters, parameterized queries, and grounded summaries over returned records. Retain record IDs for verification and cover empty and partial result sets.

**Acceptance:** “last five reports in the last hour” returns the correct records and a grounded spoken answer. **Dependencies:** T07, T09.

## Milestone 3 — Benchmarking and research dashboard

### T11 — Create the incident evaluation dataset and scoring rules (M)

Collect consented local-speech examples covering terminology, noise, missing facts, corrections, confirmation, and retrieval. Annotate expected outputs, separate development/held-out data, and define scoring. Keep larger media external with versioned manifests.

**Acceptance:** the same documented dataset/scoring procedure can evaluate multiple configurations without tuning on held-out data. **Dependencies:** T02. Start early alongside inference work.

### T12 — Replace the mock benchmark runner with an incident runner (L)

Run real versioned scenarios against the Pheme VA service through the shared Go client, including separate public-API end-to-end checks. Collect existing Rust/host stage and resource metrics, workflow outcomes, and full runtime configuration. Distinguish execution errors from output-quality failures. Remove `license-plate-monitoring` from accepted benchmark requests in the Go code and tests when this ticket is implemented; interview development remains deferred.

**Acceptance:** experiments produce measured workflow results, identify unavailable metrics, and no longer accept the removed use case. **Dependencies:** T10, T11.

### T13 — Persist experiments and expose dashboard query APIs (M)

Add a Go-owned SQLite experiment/result repository, paginated history and filters, and defined interrupted-run handling after restart. Keep experiment storage separate from Pheme incident/session storage and preserve existing create/get contracts.

**Acceptance:** historical runs survive restart and can be queried; interrupted runs have a defined visible outcome. **Dependencies:** T12.

### T14 — Build the benchmark dashboard (L)

Add experiment configuration/creation, history, progress polling, and result details to the web app. Show quality, latency, memory, and available device metrics with clear loading/empty/error/unavailable states.

**Acceptance:** researchers can create a run and inspect measured results without curl. **Dependencies:** T13.

### T15 — Add experiment comparison and export (M)

Compare selected models, quantization, runtimes, and devices. Display configuration differences and flag different datasets or measurement methods. Add useful charts and export with reproducibility metadata.

**Acceptance:** researchers can compare compatible runs and export results with configuration and measurement context. **Dependencies:** T14.

## Milestone 4 — Target-device evaluation

### T16 — Confirm target hardware and deployment constraints (S)

Obtain device/OS/RAM/accelerator/storage/battery details, local/offline requirements, instrumentation access, thermal conditions, and the operating workload. Clarify the illustrative 8–10-hour target and robotics scope.

**Acceptance:** device constraints and evaluation expectations are recorded; unresolved items are explicit. **Dependencies:** KLASS specifications. Request alongside implementation; do not block the prototype.

### T17 — Deploy the baseline workflow to the selected device (L)

After selection, split into device-specific tasks: service packaging for Jetson or native inference/workflow integration for iOS. Reuse schemas, prompts, and conformance scenarios. Validate the complete speech/report/retrieval loop and document setup.

**Acceptance:** the baseline runs on the actual target at the agreed execution location; remote inference is not presented as on-device execution. **Dependencies:** T10, T16.

### T18 — Measure device efficiency and evaluate optimizations (L)

Implement device measurement collection and compare a bounded set of model/runtime configurations. Measure idle/active power, energy per task, temperatures, sustained performance, and agreed endurance. Evaluate local-speech improvements on held-out examples and present quality/energy trade-offs in the dashboard.

**Acceptance:** repeated measurements, configurations, and workload definitions support a reproducible deployment recommendation; whole-device and component measurements are distinguished. **Dependencies:** T11, T12, T15, T17.

## Recommended sequence and checks

Continue from the implemented Rust/TUI/metrics foundation with T05 and T11 in parallel, and request T16 information alongside development. Then complete the Pheme-owned spoken workflow before expanding dashboard comparisons and device optimization. Do not assign calendar deadlines until team capacity and device availability are known.

For implementation tickets, keep Go unit tests beside Go packages and Rust tests beside the relevant crate. Run `gofmt`, `go vet`, and Go tests for backend changes; run Rust formatting, Clippy, and workspace tests for Rust changes; and run lint/build for frontend changes. Ordinary CI should use fakes/small fixtures. Model and device evaluations run separately with recorded configurations. For documentation-only updates, verify links, current-code claims, and consistency; no runtime tests are required.
