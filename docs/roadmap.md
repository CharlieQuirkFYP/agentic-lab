# Implementation Roadmap

These are local planning references, not Linear issue IDs. Tickets are not created in Linear. Estimates are relative: S small, M medium, L large. Only the documentation work in T01 is covered by this update; other capabilities remain planned.

See [requirements](requirements.md), [architecture](architecture.md), and [benchmark methodology](benchmark-methodology.md). Build incrementally, retaining existing public contracts unless a ticket explicitly changes them.

## Milestone 1 — Real Local Incident Analysis

### T01 — Update project scope and architecture documentation (S)

Update AGENTS.md and README for the student-built incident workflow, removed Use Case 3, Whisper/llama.cpp baseline, and both web roles. Document requirements, architecture, evaluation, and delivery sequence; distinguish implemented behaviour and pending decisions.

**Acceptance:** documentation consistently reflects the KLASS direction and links to detailed plans. **Dependencies:** none. **Status:** covered by this documentation update.

### T02 — Define incident, conversation, and AI service contracts (M)

Define required fields, unknown values, session states, draft revisions, clarification/confirmation rules, turn requests/responses, and Python contracts. Include complete, incomplete, corrected, cancelled, and confirmed examples. Specify time-filter semantics and retry handling; preserve the existing text-analysis contract.

**Acceptance:** Go and Python can be implemented against explicit schemas and expected conversation outcomes. **Dependencies:** T01.

### T03 — Create the Python AI service foundation (S)

Add validated HTTP schemas, health/readiness endpoints, runtime/path/timeout configuration, tests, Python CI, and local setup instructions. Keep weights outside Git.

**Acceptance:** the service starts locally and validates requests; tests do not require downloaded models. **Dependencies:** T02.

### T04 — Implement local incident analysis through llama.cpp (M)

Add the runtime adapter, structured extraction prompts, output validation, and timeout/unavailable-runtime handling. Choose a provisional model using the development examples; record model, quantization, prompt/runtime revisions, and stage timing.

**Acceptance:** Python returns real structured reports and handles invalid model output and runtime failures. **Dependencies:** T03.

### T05 — Connect the Go incident analyzer to Python (S)

Implement PythonIncidentAnalyzer behind the existing interface, configure concrete wiring in main, propagate cancellation, and preserve simple HTTP errors. Add adapter tests and a cross-service smoke test.

**Acceptance:** the existing text endpoint performs real local analysis without changing its contract; mock mode remains usable. **Dependencies:** T04.

## Milestone 2 — Complete Voice Reporting Workflow

### T06 — Implement conversation state and human confirmation (L)

Add session/turn APIs and Go state transitions, using a small in-memory implementation before durable storage. Support clarification, correction, readback, cancellation, and confirmation of the current revision. Deduplicate retried turns.

**Acceptance:** text scenarios complete the workflow; stale or ambiguous confirmation cannot finalize a report, and corrections invalidate prior confirmation. **Dependencies:** T02, T05.

### T07 — Persist sessions and confirmed incident reports (M)

Add SQLite migrations and repositories for sessions, turns, and reports. Separate occurrence, recording, and confirmation timestamps. Make finalization atomic and idempotent.

**Acceptance:** reports survive restart, sessions resume consistently, and retries cannot create duplicates. **Dependencies:** T06.

### T08 — Add Whisper transcription and audio turns (M)

Implement a replaceable transcription adapter, provisionally whisper.cpp. Accept/normalize supported audio formats; enforce limits and handle silence, cancellation, and invalid input. Record runtime configuration and transcription duration.

**Acceptance:** audio enters the same conversation workflow as text and failures are recoverable. **Dependencies:** T03, T06.

### T09 — Build the web voice console and spoken responses (M)

Add push-to-talk, speech output through an adapter, listening/processing/speaking status, and development views of transcript/draft/state. Route spoken corrections and confirmation through the application workflow. Document speech-synthesis execution location.

**Acceptance:** a user reports, corrects, and confirms an incident through spoken turns; offline/local claims match the implementation. **Dependencies:** T07, T08.

### T10 — Implement incident retrieval and spoken summaries (M)

Add listing/detail APIs and validated time/count filters with parameterized queries. Interpret spoken requests and summarize only retrieved records, retaining IDs for verification. Cover empty and partial result sets.

**Acceptance:** “last five reports in the last hour” returns the correct records and grounded spoken answer. **Dependencies:** T07, T09.

## Milestone 3 — Benchmarking and Research Dashboard

### T11 — Create the incident evaluation dataset and scoring rules (M)

Collect consented local-speech examples covering terminology, noise, missing facts, corrections, confirmation, and retrieval. Annotate expected outputs, separate development/held-out data, and define scoring. Keep larger media external with versioned manifests.

**Acceptance:** the same documented dataset/scoring procedure can evaluate multiple configurations without tuning on held-out data. **Dependencies:** T02. Start early alongside inference work.

### T12 — Replace the mock benchmark runner with an incident runner (L)

Run real versioned scenarios through the application workflow. Collect stage/resource metrics, outcomes, and full runtime configuration. Distinguish execution errors and output-quality failure. Explicitly remove licence-plate monitoring from supported benchmark requests and update affected tests/API docs; interview development remains deferred.

**Acceptance:** experiments produce measured workflow results, identify unavailable metrics, and no longer accept the removed use case. **Dependencies:** T10, T11. Basic timing instrumentation starts in T04/T08.

### T13 — Persist experiments and expose dashboard query APIs (M)

Add a SQLite experiment/result repository, paginated history and filters. Preserve create/get contracts and define interrupted-run handling after restart without assuming the in-memory queue is durable.

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

Begin with T01–T05, start T11 after contracts are defined, and request T16 information in parallel with development. Then complete the spoken workflow before expanding dashboard comparisons and device optimization. Do not assign calendar deadlines until team capacity and device availability are known.

For implementation tickets, use meaningful Go/Python unit tests at integration/state boundaries and shared cross-service scenarios. Run gofmt, go vet, and Go tests for backend changes, and lint/build for frontend changes. Ordinary CI should use fakes/small fixtures; model and device evaluations run separately with recorded configurations. For documentation-only updates, verify diff whitespace, links, current-code claims, and consistency; no runtime tests are required.
