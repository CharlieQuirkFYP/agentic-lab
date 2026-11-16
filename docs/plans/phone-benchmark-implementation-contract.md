**Implementation contract — phone benchmarks, 2026-11-10**

Active coordination contract for the approved implementation. Preserve existing legacy metric uploads and overview. Canonical source types and report logic live in `shared/phone-benchmarks/`; use snake_case on the wire. All new inference executes through the existing on-device Pheme bridges.

**Immutable observation envelope**

`POST /api/v1/phone/observations` accepts `{schema_version:1, device, records:[record]}`. Device retains existing `installation_id, hardware, os_version, app_version, build`. A record is:

```typescript
interface PhoneObservation {
  schema_version: 1;
  record_id: string;
  producer_stream_id: string;
  record_sequence: number;
  timestamp_ms: number;
  kind: 'configuration' | 'conversation' | 'turn' | 'attempt' | 'metric_batch' | 'sweep' | 'job' | 'finalization';
  entity_id: string;
  revision: number;
  context: {
    configuration_id?: string;
    conversation_id?: string;
    turn_id?: string;
    attempt_id?: string;
    sweep_id?: string;
    job_id?: string;
    runtime_session_id?: string;
  };
  data: Record<string, unknown>;
}
```

IDs use random UUIDs, scoped by installation. A new revision is a new immutable record; latest entity revision controls projection. Reject conflicting reuse of record ID, stream/sequence, or entity/kind/revision for one installation. Identical retries acknowledge the same records. References may arrive out of order and remain unresolved until parents arrive. Old revisions cannot resurrect finished work. Each upload chunk is atomic, bounded, and returns `{status:'accepted', record_count, accepted_record_ids:[...]}`. No transcript/audio/reply text or freeform errors in observation producers.

`configuration.data`: `{asr:{id,name,sha256,...}|null, reasoning:{id,name,sha256,...}|null, settings:{...effective settings}, runtime:{...}, role_sha256?:string}`. Entity ID is immutable configuration hash/ID. Preserve both model names and hashes. Metadata missing from a legacy upload remains unknown.

Lifecycle `conversation`, `turn`, `attempt`, `sweep`, `job` data uses `{status, started_at_ms?, finished_at_ms?, source?, stage?, turn_index?, case_id?, repetition?, reason_code?, input_kind?, native_run_id?, duration_ms?, ...}`. Status values are `planned|running|succeeded|failed|cancelled|interrupted|skipped`; conversations may also be `active` while awaiting another manual turn. Allowed sources are `manual|sweep|acceptance`. Stage is `transcription|reasoning|loading|warmup`. Never infer terminal outcomes from missing records. Manual conversation stays active until New Chat/reset/loss of native context.

`metric_batch.data` is `{batch:<existing v1 metric batch>}`. Bind its context to the pre-registered attempt; maintain native run association. Metrics include original name/unit/scope/source/value/unavailable_reason. `finalization.data` contains `{status:'complete'|'partial', record_count?, event_count, dropped_events, reason_code?}` with conversation/attempt/sweep context. This is telemetry completeness, independent of execution status.

**Read and overview endpoints**

Add `GET /api/v1/phone/observations?limit=100&cursor=...&as_of=...` with optional `installation_id, conversation_id, sweep_id, configuration_id, since_ms, until_ms`. It returns `{schema_version:1, as_of, records:[storedRecord], total, next_cursor}`. `storedRecord` is `{device, record, received_at_ms}`. Cursor/watermark must yield a consistent finite snapshot. Add lightweight conversation/sweep projections if needed; do not make clients reconstruct from only one page.

Add `GET /api/v1/phone/overview` with the same applicable filters. API agent owns the exact overview DTO and must send it to web/shared agents early. It must summarize the entire selected dataset, preserve legacy observations, give current/recent explicit conversations plus model metadata, observed outcome counts, grouped numeric/status summaries, last receipt/coverage and source/completeness. Preserve legacy unknowns; no invented chat counts. The existing `GET /phone/metrics` and experiment endpoints stay compatible.

**Shared aggregation API**

Shared module owns types above plus pure `projectObservations(storedRecords)` and JSON/CSV/Markdown report generation. Preserve relationships, all outcomes, latest entity revisions, identity conflicts, source/scope/unit/stage/configuration separation, valid zeroes, missing readings and distinct installation IDs. Initial p95 minimum is 20 eligible samples per compatible series. No browser/Node/native globals in aggregation code. Export from `shared/phone-benchmarks/index.ts`; platform code owns persistence/network/files.

**Phone ownership interfaces**

Phone store/controller agent owns `ios-app/src/benchmarkStore.ts` (durable archive, immutable observation creation, delivery queue), `benchmarkCollector.ts` (correlation/native drain), manual integration in `localChat.ts`, `onDeviceChat.ts`, `phoneMetrics.ts`, `_layout.tsx`, `index.tsx`, and local overview UI. Provide injectable store API to runner and notify others of final signatures early. One root collector drains native data. Do not consume it from the runner independently.

Runner agent owns `ios-app/src/benchmarkRunner.ts`, `benchmarkDataset.ts`, `app/benchmark.tsx` and runner/dataset tests. It uses store/controller APIs, existing Pheme APIs and later native context/keep-awake additions. Persist matrix before execution; persist attempt registration before native call; await native settlement and outcome persistence before next job. Automatic inference retries default off. Maintain context within a case and reset between cases. Protect corpus assets from recording cleanup. Provide a bounded on-device acceptance entry (URL parameters or launch configuration) using only real local audio/model artifacts; agree its interface with device agent.

Native agent owns Swift runtime/bridges and JS native type/client APIs plus additive Rust FFI identity changes. Must publish API signatures to phone/runner agents promptly. Resource samples originate from native iOS and are bounded; unavailable power/energy/GPU stay explicit. Effective settings and runtime identity must be inspectable.

Root coordinates server lifecycle, final integration, shared build resource scheduling and evidence-backed reporting. Device agent owns command-line device readiness/acceptance scripts. Packaging agent owns model bundle/prepare scripts. No concurrent full native builds until source and packaging changes are integrated.
