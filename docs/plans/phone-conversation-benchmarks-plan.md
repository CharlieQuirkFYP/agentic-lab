**Phone conversation metrics and autonomous benchmark plan**

Status: core implementation and physical pipeline verification recorded,
2026-11-11. The archive, native correlation/sampling, phone sweep runner, Go
observation API, additive dashboard and shared reports are implemented. The
physical 24-job matrix completed 15 jobs and retained nine configuration failures;
all 2,159 records match Go. A real Go outage plus forced app restart recovered
six jobs with the interrupted execution retained and exact 644-record parity.
See the [physical acceptance record](../testing/phone-benchmark-acceptance-2026-11-11.md)
for evidence and remaining model reliability, calibration, quality and endurance
gates. The sections below retain the approved design and its original baseline;
they must not be read as a claim that every research acceptance gate passed.

The outcome is: record several conversations on an iPhone, select available model configurations, run the same conversations repeatedly on that phone, see an overall metrics dashboard of everything collected so far, inspect metrics under each conversation, and export a reproducible comparison report. Ordinary manual chats use the same conversation metrics model. The overall dashboard is a primary view throughout delivery, alongside conversation and sweep drill-downs.

Delivery follows two tracks: **A gives existing uploads a useful grouped dashboard immediately; B establishes explicit conversation ownership and durable phone collection alongside the autonomous runner.** B's contracts are designed before A's view model is finalized, so the UI can transition without another redesign.

**1. Verified starting point and scope**

The following observations come from the current working tree, including its existing uncommitted phone-metrics changes:

- The TUI's actual conversation structure is `ChatSnapshot -> ChatTurn -> ChatRun`, with retained events and metric summaries. See [core chat types](../../pheme-va/crates/core/src/chat.rs), [runtime conversations](../../pheme-va/crates/runtime/src/conversations.rs), and [versioned TUI archives](../../pheme-va/crates/cli/src/tui/history.rs). The separate `TelemetryStore::RunReport` is not a complete model of a multi-turn chat.
- Phone [chat history](../../ios-app/src/chatStore.ts) currently persists individual turns. Successful reasoning history remains in Rust; reopening a saved chat does not restore that native context.
- [Phone metadata producers](../../ios-app/src/phoneMetricEvents.ts) report successful ASR metadata and reply timing. Native ASR events can also describe failures. Complete conversation/turn lifecycle reporting is missing.
- The root collector drains native metrics into a SQLite outbox. Current limits are 512 retained native events and 2,000 queued batches / 8 MiB. Acknowledged batches leave the outbox; it is not a long-term phone metrics archive. See [collector](../../ios-app/src/phoneMetrics.ts), [sync](../../ios-app/src/metricsSync.ts), and [outbox](../../ios-app/src/metricsOutbox.ts).
- ASR models can be imported. Reasoning selection currently reads bundled models only; [bundling](../../ios-app/scripts/bundle-models.sh) currently selects a fixed ASR/reasoning baseline. Multiple reasoning choices cannot be assumed to exist on the installed phone.
- Go ingests phone observations without launching inference. Existing identifiers group by installation and native/host run, not by conversation. The dashboard can display/export these observations, but its phone fetch currently reads only the first page.
- Native CPU/RAM, battery and thermal sampling are not implemented. Power/energy are unavailable. Existing timing uploads do not supply those measurements.

Core delivery includes an overall metrics/progress dashboard, conversation grouping, durable attribution, local history, an offline-capable sweep runner, reports, and physical-device verification. A distinct native resource milestone adds obtainable phone measurements. Verified watts/joules, GPU instrumentation, full incident workflow, and automated reply-quality scoring remain separate capability gates; unavailable values are explicitly represented.

Preserve existing working-tree edits during implementation and deliver the milestones as reviewable changes. Current wire/API details are in [the implementation contract](phone-benchmark-implementation-contract.md) and [phone observation API](../api/phone-observations-api.md).

**2. User workflow and visible result**

1. In normal chat, start a conversation, speak multiple turns, review transcripts and receive replies. Open that conversation's metrics to inspect each turn, stage and metric series. New Chat closes the previous collection session and resets native conversation context after active work settles.
2. In a phone Benchmark screen, record or import utterances, arrange them into ordered conversations, and save a versioned dataset. Give datasets and cases neutral identifiers; spoken content stays on the phone.
3. Select compatible ASR and reasoning configurations, repetitions and a recorded protocol. Review the number of planned conversation executions, model availability and required storage before starting.
4. The phone performs the queued conversations sequentially, with visible progress, pause and cancellation. In benchmark mode it explicitly submits each model-produced transcript automatically; normal chat keeps transcript review.
5. Open Overall Metrics to see collected results, progress, metric trends and recent activity without selecting a conversation. Inspect completed or interrupted conversations on the phone, including while offline. Sync when desired, inspect the same grouping on the web, and export JSON, CSV or Markdown.

Overall Metrics is the default metrics landing view. It covers both manual chats and sweeps, with links into these detail views:

```text
Sweep (optional; manual chats exist independently)
  Model configuration
    Conversation execution (one dataset case, one repetition)
      Turn
        Transcription attempt -> grouped metrics and raw samples
        Reasoning attempt     -> grouped metrics and raw samples
    Conversation summary
  Configuration comparison
Sweep report
```

A conversation summary links to all its turns. A metric row shows its unit, source and scope; numeric values have latest/min/mean/max/count, while status values remain readable text. Selecting a metric reveals its sample history. Unavailable samples produce gaps and reasons, including broken chart segments across gaps.

**2a. Overall Metrics — results and progress so far**

Preserve and expand the current dashboard's Metrics overview. Deliver its web version in M1; add the phone-local version with M2's archive. Conversation selection opens detail without silently changing the overall dashboard's filter scope. Provide three sibling destinations: Overall Metrics, Conversations and Sweeps; a conversation need not belong to a sweep.

| Visible area | Required content |
| --- | --- |
| Coverage and freshness | Selected period/devices, retained data coverage, last collection/receipt time and live/reconnecting/stale status. |
| Work so far | Conversations, turns and stage attempts observed; completed, running, failed, cancelled, interrupted and skipped outcomes; configurations tested. Distinguish executions from planned jobs and raw batches. |
| All metric families | Timing, CPU/memory/resources, battery/thermal, power/energy, counts and statuses, with every observed series discoverable and unsupported planned measurements labelled unavailable. |
| Numeric summaries | Latest value with timestamp, min/mean/max, valid observation count and unavailable count/reasons. Percentiles appear only where the report's sample/unit rules are met. |
| Trends and comparisons | Time-series charts, per-configuration series and stage filters, with links to contributing conversations/attempts. Show numeric distributions separately from cumulative work counts. |
| Recent activity and active work | Most recent conversations/attempts, their model configuration, outcome and timing; active sweep progress against its planned jobs. |
| Data health | Partial/unlinked observations, dropped/truncated samples and pending/blocked uploads where the source actually reports those states. |

Default to all retained phone-origin history, explicitly labelled with its date range and coverage. Offer Today, a custom date range, device/installation, model configuration, manual/sweep, stage and outcome filters; explicit filters apply consistently to cards, charts, activity and exports. Keep the selected context visible when opening and returning from a detail view. Organize all metrics by category with search and expandable groups rather than silently selecting a fixed handful. On the phone use the same information in a compact scrollable layout.

An overall view can show activity counts across all selected devices/configurations. Performance series remain separated by compatible device/build/configuration/dataset/protocol and metric source/scope/unit; no unexplained average blends different model combinations or desktop/mock observations. In M1, legacy metrics may only support summaries labelled as reported samples with unknown configuration. Unknown metadata remains a visible group, never borrowed from the current model selection. Conversation and outcome counts unavailable in legacy data remain unknown; batch/experiment counts are not renamed as chat counts.

Overall totals must describe the entire selected retained dataset, even if the activity table displays only 20 rows. Add a read-only overview query in M1 (proposed `GET /api/v1/phone/overview`) using the existing Go metrics/phone services and SQLite data. Return coverage, an as-of revision/time, supported counts and mergeable series summaries over the full filter scope, independently of paginated detail rows. Reuse existing summary semantics and shared reporting rules; M1 does not need a second percentile implementation. If overview metric groups require pagination, expose that coverage and make every group reachable. Do not sum conversation summaries and their underlying attempts/batches as separate observations. Extend this query for B's explicit identities as M2 ships.

The phone overview updates from durable local archive changes, including while offline. The web overview updates after server receipt using coalesced existing dashboard notifications and a fresh overview snapshot; reconnect triggers a refresh. Every refresh uses consistent filters/revisions and prevents an older response from replacing a newer one. Aggregate/stat changes, new metric series and recent activity must become visible without reopening the screen. UI refresh work must remain bounded during inference.

Distinguish "collected on this phone" from "received by the server". Unsynced local work appears immediately on the phone; the web can display only received records and any last-reported sync status, labelled with its age. It must not invent a current phone queue size or imply that stale server totals include offline activity. Archive cleanup changes retained-history coverage; never call those counts lifetime totals without separate durable lifetime accounting.

Exit gate: from the default overview the user can see all recorded metric categories and how much work is represented, filter to one configuration and drill into contributors. More than 100 batches and multiple pages of conversations do not truncate headline totals. Offline work, delayed uploads, reconnects and duplicate acknowledgments update only the appropriate local/server totals. Unknown/unavailable data, valid zeroes and historical partial coverage remain visible.

**3. Identity, boundaries and ownership — contract agreed before implementation**

| Identifier | Meaning and rule |
| --- | --- |
| `installation_id` | Existing phone installation identity; included in every join and uniqueness constraint. Hardware name is not identity. |
| `runtime_session_id` | New UUID per native runtime lifetime; disambiguates native counters after reload/restart. |
| `sweep_id` | UUID for a persisted test campaign; absent for manual chat. |
| `configuration_id` | Hash of canonical model artifact hashes, prompts/role hash, runtime/build and effective inference settings. Store the complete manifest alongside the hash. |
| `dataset_id` / `dataset_revision` | Stable dataset identity and hash of ordered case/turn/audio manifests and applicable scoring references. |
| `case_id` / `repetition` | Which scripted conversation and repetition a job represents. |
| `conversation_id` | UUID for one actual conversation execution. Repeating or retrying a case creates a new ID, linked to the original job/execution. |
| `turn_id` / `turn_index` | Stable UUID and order within that conversation. |
| `attempt_id` | UUID for one admitted stage attempt, persisted before invocation. Native ASR-internal recognizer retries retain their own attempt index under this stage attempt. |
| `native_run_id` | Original engine run identity, explicitly associated with the attempt; never parsed to infer new conversation ownership. |

One planned job is `(sweep, configuration, case, repetition)`. A retry adds an execution; it does not overwrite the job's previous evidence. Record model-loading and warm-up attempts separately from measured turns.

Manual conversations start on the first admitted turn after New Chat/reset. Finish/New Chat, model reconfiguration, or loss of native context ends the current context segment. For the initial implementation, changing a model ends the conversation and starts another: no silent continuation with different history/configuration. A process crash leaves an interrupted conversation; reopening its archive is read-only. Navigation alone does not end a live conversation. Idle wall time is retained but excluded from active-inference totals.

Phone lifecycle metadata has explicit started/finished timestamps and outcomes. Do not infer start/finish from the first/last uploaded event. Durations use a monotonic phone clock; wall-clock timestamps support display and traceability. Record the timer boundary for each measurement.

Pheme owns successful model context, validation, reset and cancellation semantics. The phone records observation ownership and schedules benchmark invocations through the existing native bridges. Go stores observations, derives read views and generates reports; it does not maintain a second incident/conversation inference state machine. The existing generic Go mock runner stays separate. On-device execution is an explicit target-specific benchmark requirement.

Reuse the TUI's stage/summary semantics and portable types where appropriate. Create a metadata-only projection for uploads rather than serializing `ChatSnapshot` verbatim: that snapshot contains transcript/reply text. No dependency on the desktop CLI or desktop process sampler is introduced into mobile builds.

**4. Track A — useful grouping of historical uploads**

Implement a pure web grouping library, metric-series components and a paginated loader, then integrate them alongside the expanded overall dashboard. M1 also adds the read-only overview query described in section 2a so global summaries do not depend on loaded detail pages; the upload contract stays compatible. Separate the ingestion adapter from the presentation model so B can populate the same components later.

Correlation rules:

- Obtain installation attribution from each batch's experiment detail, cached by experiment ID. Existing `/phone/metrics` batches alone do not contain the device record needed for cross-container joins. If attribution cannot be obtained, keep that container isolated.
- Within one installation, join host ASR metadata and reply observations using explicit `turn_id` event values; attach native ASR batches using the metadata's explicit `native_run_id`. Prefixes are only a legacy recognition fallback, not sufficient evidence for a merge.
- Preserve original experiment/run/batch identities. Deduplicate an exact batch by `(experiment_id, run_id, sequence_number)`; conflicting duplicates are errors, not discarded silently.
- Missing ASR does not prove a typed turn; missing reply does not prove a failed or still-waiting turn. Use labels such as `reply observed`, `ASR observed`, `partial`, or `unlinked` unless a recorded status proves more.
- Existing uploads do not establish multi-turn conversation boundaries. Display reconstructed turns in a clearly labelled historical area; never synthesize entire conversations from proximity, model choice or timestamps.
- Keep metadata/status series available. Compute numeric summaries only from finite numeric samples, separating `(stage, name, unit, scope, source)`. Retain valid zeroes and all unavailable reasons.

Load bounded time ranges/pages with cancellation, cached detail fetches and bounded concurrency. Offer Load more and show loaded coverage. Do not fetch the entire lifetime history on every WebSocket event. Coalesce refreshes and discard stale responses. Existing offset pagination can shift under live ingestion; label live historical views as provisional and do not certify a complete sweep from them.

Exit gate: fixtures spanning more than 100 batches, out-of-order arrival, two installations with identical native IDs, missing metadata, native failure events and repeated uploads all display without incorrect joins or inflated counts. Overall summaries cover the complete selected dataset independently of detail pagination. Existing exports and phone ingestion remain compatible.

**5. Track B — durable phone collection and explicit transport**

Persist conversation, turn and attempt registration before invoking native inference. Snapshot both intended configuration and the actual applied configuration, including model hashes, language/dictionary hash, ASR/reasoning threads, prompt/role hash, context/output limits, sampling parameters and model/runtime/app build identifiers. Hashes must describe effective values, including native defaults.

Extend the native boundary additively so the preallocated attempt ID can be associated with the native run before the first event is emitted. Prefer an explicit request/run identity accepted at admission, with matching identity in status, errors and results. Keep existing ABI entry points compatible. If an additional versioned entry point is needed, add it to the header and real Swift C-ABI tests. Do not rely on JavaScript inspecting whichever run happens to be active at poll time, or attach identity only on success.

Maintain one root-owned collector and one app-level scheduler for all manual/sweep work. Swift's existing native admission remains the final guard. Model switches, manual turns and sweep jobs cannot interleave; UI controls communicate that ownership. Native collection remains responsive during inference.

Use explicit outcomes: planned/running/succeeded/failed/cancelled/interrupted/skipped, with bounded structured reason codes. An attempt is terminal only after native work settles, except recovery can mark a previously running attempt interrupted when its runtime session is gone. Record cancellation requested separately from completed cancellation; do not advance to another native job while the previous one is still settling.

Local SQLite changes:

- Add indexed metadata for conversation/turn/attempt ownership, dataset manifests, sweep jobs and execution progress. Preserve existing saved turns/transcripts; migrate them as legacy records without inventing conversation boundaries.
- Retain immutable observation records and metric batches in a local archive. Use one transaction for archiving a drained batch and registering its pending upload. Prefer a delivery-status/reference table over a second full payload copy for new records.
- Keep the current v1 outbox until its queued batches are acknowledged. Never rewrite a pending payload or upload it again under a different identity during migration.
- After successful native completion, commit outcome, associated metrics and job progress before scheduling the next operation. Persist an explicit collection-finalized record with sample/batch counts and drop/gap accounting after the final drain.
- On restart, running work from a vanished runtime becomes interrupted. Acknowledged records remain available for local drill-down until retention removes a whole eligible conversation.

Retention and backpressure are admission rules, not merely alerts. Configure byte/count budgets for the archive, pending uploads and corpus separately. Reserve space for the bounded next operation plus finalization records using configured duration/sample limits. Never evict active, pinned or unsynced records automatically. At capacity, pause new work and offer sync/export/delete of eligible completed data. Sweeps default to pinned retention until exported or explicitly released. Initial budgets must be measured against the acceptance corpus; the old 8 MiB queue cannot be assumed adequate for resource samples.

A native-to-SQLite crash window still exists. Record last durable sequences, observer drop counters and finalization counts; mark truncated/incomplete attempts rather than promise zero loss. A final result received before its outcome transaction commits is interrupted/uncertain after a crash, not silently successful.

Transport design: retain `/api/v1/phone/metrics` unchanged for v1. Add an explicitly versioned observation envelope at proposed `POST /api/v1/phone/observations` with installation/build attribution and bounded immutable records: lifecycle, configuration, run association, metric-batch and collection-finalized records. Metric batches retain the existing metrics v1 schema; only the new envelope carries relationships.

Each record has `(installation_id, producer_stream_id, record_sequence)` identity and immutable content. The server atomically accepts an upload chunk; identical retries acknowledge the same records, conflicting content returns a conflict. Device/profile attribution is preserved for each execution across app updates. Missing parents may arrive later and remain visibly unresolved; contradictory associations fail validation. A completed marker alone is insufficient for complete-report status if referenced records or sequences are missing.

Implement validation/domain projection in a Go service, transaction/uniqueness logic in its SQLite repository, and HTTP limits/errors in handlers. Add read routes for phone conversations and sweeps, with installation filters and cursor pagination. Keep legacy experiment containers intact and provide links to them. New observations are not dual-written as both legacy and new measurements; reconciliation fixtures prevent double-counting across adapters.

Deploy the compatible server first. An older server rejecting the new route leaves new records queued and displays an upgrade-required state; do not drop relationships by silently downgrading uploads.

Exit gate: manual multi-turn chat persists and syncs as one conversation; native errors and early cancellation have ownership; late uploads do not resurrect interrupted work; old queued batches still upload; crash/retry/out-of-order tests preserve identities.

**6. Dataset and model preparation**

Create a dedicated corpus store, separate from recoverable microphone recordings. Existing recording cleanup can remove prior recordings; benchmark assets must be pinned independently while referenced by a dataset or active sweep.

A dataset manifest contains ordered conversations and turns, stable IDs, audio checksums, audio duration/format, optional local reference transcripts and a revision hash. Recording/import verifies WAV decoding and the native configured duration limit (currently at most 120 seconds per utterance), rather than relying on the recording store's broader limit. Copy imported files into managed local storage and verify they remain available after picker access ends. Freeze a dataset revision for each sweep; edits create a new revision.

Support a reproducible fixed script: follow-up utterances are identical across model combinations even when replies differ. Label this evaluation mode; it is not an adaptive dialogue or proof of incident task success. An ASR-only mode is explicit and produces no invented reasoning measurements.

First support multiple checksum-verified bundled reasoning models through a manifest-driven bundle selection; retain existing ASR import. Update packaging checks to validate every selected artifact and license/role resource. On-phone reasoning import can be a later convenience and is not a prerequisite for the first matrix. No downloads or model import run inside a measured trial.

Preflight checks artifact availability, combined disk budget, compatible runtime, valid effective settings, local storage reservation, enabled collection and native readiness. A model can still fail to load at runtime; report that as a configuration failure, never substitute a smaller/default model. A fresh Release build and device installation are required for bridge or bundled-model changes.

Exit gate: at least two compatible ASR and two compatible reasoning artifacts can be explicitly selected in one installed phone build; the pinned corpus survives normal recording cleanup and app restart.

**7. Autonomous runner and experimental protocol**

Persist the job matrix before starting. Expand `configurations x conversation cases x repetitions`, with a stable job order and randomization seed in the sweep manifest. Execute only one native operation at a time. The pure scheduling logic takes injected native/storage/clock dependencies for deterministic tests.

Default protocol:

1. Randomize configuration order in reproducible blocks across repetitions. Reuse warm engines within a configuration block; do not use a permanently fastest-first ordering that hides drift.
2. Verify/apply the selected pair and record model loading separately. Run an explicitly recorded warm-up outside measured results, then reset conversation context.
3. Apply the recorded cooldown policy between blocks. Before native thermal support, use a fixed recorded interval and label thermal state unknown. With that support, require the configured thermal state for a stable interval or pause at a bounded deadline. Neither policy proves identical silicon temperature.
4. Reset native context before each independent conversation; keep it between that conversation's turns. For every turn, persist attempt identity, transcribe its fixed recording, submit that actual transcript, collect the reply and finalize the observation. Silence/invalid input is recorded according to the case's expected outcome; it is not quietly excluded.
5. A failed stage terminates that conversation execution by default; remaining dependent turns are skipped with reasons. Continue the next independent case only after engine health/admission is confirmed. Automatic inference retries default off; requested reruns create a new conversation execution and retain the original failure.
6. Finalize counts and collection completeness, then continue. Model/configuration failures mark affected jobs explicitly, while infrastructure failures such as storage loss pause the sweep.

Pause means finish the current conversation and stop before the next; Cancel requests native cancellation and marks the current execution accordingly. Backgrounding or memory-pressure teardown pauses scheduling and follows native settlement rules. Keep the display awake only while a sweep is actively executing, release that state on every exit path, and test that the mechanism does not imply permission for iOS background execution.

After process loss, do not resume the next turn with empty context. Mark the prior execution interrupted; resume at a new conversation execution from its first turn, or skip it explicitly. A restart may require reloading and warming models; those operations remain outside measured results.

Network is optional. The default measured protocol defers uploads until a conversation/block ends or the sweep completes, subject to reserved storage. Persist the upload policy, screen brightness/power mode/charging conditions and sampler configuration as test conditions. Live-upload mode is allowed but distinguished in comparisons. App UI rendering and debug logging must not grow with all retained samples during a timed operation.

Exit gate: the fake runner produces the exact planned matrix, maintains within-conversation history, resets between conversations, never silently substitutes models, and pauses correctly for lifecycle/storage/cancellation events.

**8. Phone-native measurement milestone**

Timing collection is the first usable release. Add the following providers under the Expo module's Swift host, keeping portable measurement contracts in Pheme:

| Measurement | Provider/boundary to validate | Report semantics |
| --- | --- | --- |
| Stage and active turn latency | Monotonic timers around native operations and existing core stages | Keep stage boundaries explicit; nested stages are not summed as additional elapsed time. |
| Process CPU | Supported native task/thread CPU accounting sampled as deltas | Record sampling interval and whether 100% means one core; include app/collector overhead. |
| Process memory | Native resident/physical-footprint accounting as supported | Name the accounting method; report sampled peak, not a guaranteed instantaneous peak. |
| Battery level/charging | Public iOS battery observations | Coarse device-level state/delta, not joules or per-turn power. |
| Thermal state | Public iOS thermal state | Categorical state/duration, not a Celsius reading. |
| GPU utilization, watts and joules | No verified provider in the present app | Unavailable with reason until a separately validated provider exists. |

Use a native sampler that can operate while JavaScript is busy and attach samples to the pre-registered active attempt. Start with a recorded 500 ms cadence plus operation-boundary samples, then validate overhead and bounds on the device. Short operations without enough CPU samples report unavailable rather than a fabricated zero. Session/idle samples are distinct from inference-stage samples.

Sample device-wide properties once, referencing them from applicable intervals rather than counting them independently for each nested stage. Preserve gaps, actual timestamps and source/scope/unit. Phone processes in shared native libraries remain one app process; do not claim separate engine CPU/RAM without an actual isolation method.

Measure collector overhead with bounded matched runs on the same phone, report it and set an acceptance threshold based on the pilot before publishing comparisons. Use Instruments on the attached device for sanity checks where available; Mac CPU readings never substitute for phone measurements. Validate public API/build availability during this milestone rather than promise access from an untested API assumption.

This milestone is required before describing the delivered report as containing phone resource measurements. It does not block Track A or the first timing-only sweep. A requirement for validated whole-device energy would be a separate explicit instrumentation gate; no battery-percentage conversion is used as a shortcut.

**9. Reports, statistics and completeness**

The [report template and testing walkthrough](phone-benchmark-report-template.md) defines the initial report layout, acceptance matrix and evidence fields. It is marked not run; the planned 24-conversation workload is not a measurement result.

Use a small pure TypeScript aggregation/export module under proposed `shared/phone-benchmarks/`, compatible with both web and React Native, for new canonical records. Keep it free of Node/browser/native APIs; platform adapters own fetching, persistence and saving exports. Track A's legacy adapter feeds the same presentation contracts with its limitations. Go returns validated, indexed records and completeness information; avoid independently implementing a second set of percentile formulas in handlers. Verify Vite/TypeScript and Expo/Metro resolution and add this shared path to both Web and iOS CI filters so shared logic changes cannot bypass their checks.

Every report has dataset/configuration manifests, device/build attribution, protocol revision, collection coverage, generation time and an input watermark/hash. Web exports read a server snapshot: cursor pages are pinned to an ingestion watermark and projections are derived from records at that watermark. Local exports use a consistent SQLite read snapshot. Later uploads produce a new report revision; the UI does not mutate an already exported report.

Export scope can be Overall Metrics for the current filters, one sweep, or one conversation. The overall export includes results collected so far across the selected retained dataset and identifies any active/partial executions. With the same input watermark and filters, its counts and summaries must match the overall dashboard; pagination of visible detail rows must not change the export. A live view may advance after export, but retains the export's as-of identity for comparison.

Statistics rules:

- Group by configuration, dataset revision, stage, metric name/unit/source/scope and compatible device/build/protocol. Show mixed-device data separately by default. Do not compare unlike scopes as interchangeable values.
- Timing distributions use one defined duration per eligible attempt/turn/conversation, not every emitted stage event. Report count, mean, median, min and max. Publish p95 only with a documented estimator and sufficient samples (initial UI threshold: 20); below that show insufficient samples. This threshold does not establish statistical confidence.
- Keep per-case results alongside aggregate results. Do not average averages without their counts. ASR retries appear as retry count and attempt-level diagnostics without inflating the number of conversations.
- Conversation active time sums non-overlapping measured operations. Conversation wall time, model loading, cooldown, human review and upload time are separate. Existing ASR end-to-end metrics are not speech-to-audible-reply latency; TTS/playback is outside the initial measured boundary.
- Show planned jobs, started executions, successful executions, failures, cancellations, interruptions and skipped jobs separately. Include reruns as additional executions and document denominators. Computational completion is not transcript accuracy or incident-task success.
- Resource arithmetic means are sample summaries; time-weighted means require timestamped intervals and gap coverage. Peaks are sampled peaks. Thermal categories and battery deltas have their own display rules, not generic numeric averaging. Unsupported values remain null with reasons.
- Finalization includes expected turns/attempts, durable record ranges, dropped-event counts, unresolved associations and truncation flags. A finished sweep with incomplete telemetry can be exported as partial but cannot receive a complete-data label.

Exports: JSON contains raw events, relationships and manifests; CSV includes explicit installation/device/configuration/conversation/turn/attempt/stage columns; Markdown contains configuration comparisons, per-conversation summaries and unavailable/partial-data notes. Protect CSV text cells against spreadsheet formula interpretation. Metrics-only exports omit recordings, transcripts, replies, freeform error messages and potentially sensitive chat titles. Use neutral labels by default. Any content-inclusive dataset export is a separate explicit user operation.

Optional ASR reference scoring can later run locally and upload only scores and scoring-version metadata. Reply/incident quality scoring and full speech-to-speech measurements are separate additions, not inferred from a successful reply.

Exit gate: the phone and web compute identical statistics from the same canonical fixture/snapshot, JSON can reproduce summaries, and CSV retains attribution. Missing data changes completeness, not values to zero.

**10. Delivery order and owned files**

| Milestone | Concrete delivery | Main touchpoints | Completion gate |
| --- | --- | --- | --- |
| M0 — shared design | Freeze identifiers, lifecycle, overview/filter semantics, metric boundaries, envelope examples and golden fixtures for A/B/runner. | This plan, `docs/api/`, fixture files | A complete conversation and an interrupted retry can be represented without inference or timestamp guessing. |
| M1 — Track A | Overall web metrics/progress view with full-scope summary query; historical turn grouping, metric drill-down, device-safe joins and bounded pagination. | `web/src/lib/`, dashboard components/pages, Go read services/handlers, web/API tests | Historical fixtures pass; overall totals cover all matching data; incomplete/unlinked data is explicit. |
| M2 — Track B | Native admission correlation, phone archive/lifecycle, compatible upload migration, Go ingestion and indexed conversation reads, local/web conversation history and overview updates. | `ios-app/src/`, `ios-app/app/`, native module/bridges, `pheme-va/crates/ffi/` and `reply-ffi/`, `api/internal/phonemetrics/`, handlers/server wiring | Manual multi-turn conversation survives archive/retry/restart checks; local overview works offline; native context remains Rust-owned. |
| M3 — repeatable inputs | Versioned recorded/imported corpus, pinned assets, multi-model bundle selection and preflight. | Phone dataset UI/store, native packaging scripts/catalog/tests | One build exposes the explicit 2 x 2 model matrix and retains its corpus. |
| M4 — autonomous execution | Persisted scheduler, protocol, progress, lifecycle controls and offline timing sweeps. | Phone runner/controller/UI, fake/native tests | Matrix and interruption tests pass; first timing-only phone sweep is traceable. |
| M5 — native resources | CPU/memory/battery/thermal providers, attribution, cadence and overhead characterization. | Expo-owned Swift sources, bridge types, metric contracts/tests | Each supported metric is verified as phone-origin, with documented scope and overhead. |
| M6 — reports and release acceptance | Canonical overall/sweep/conversation snapshot exports, configuration comparisons, parity tests and physical-phone acceptance record. | Shared TS reporting code, web/phone UI, Go snapshot queries, docs | Reconciled device matrix, failure scenarios and overview/detail/report parity pass. |

M0 precedes the view contract. M1 ships first; M2 follows the same contract. M3 and M5 can be developed independently after M0, while M4 requires M2/M3. M6's report logic can start with fixtures early, but its acceptance requires the completed phone workflow. This sequencing describes dependencies and does not require multiple agents.

Changes are likely needed in Swift/Rust and Go, not only TypeScript. Do not promise an all-JavaScript implementation or a fixed few-day schedule. Estimate each milestone after M0 fixtures and the first native correlation/model-load spike; model compatibility and physical-device acceptance are material dependencies.

**11. Verification matrix and final acceptance**

Automated tests use local fakes and small fixtures. Cover identity collisions across installations/runtime restarts; metadata arriving after samples; native failures before a result exists; duplicate/conflicting uploads; more than one page of data; full-scope overview totals independent of detail pagination; filter consistency; stale refresh rejection; newly appearing metric series; zero/unavailable/string values; model reconfiguration; warm-up exclusion; internal ASR retries; outbox/archive transaction failures; protected dataset assets; cancellation settlement; process-loss recovery; pending v1 migration; and upload-before/after-ack crashes. Test callbacks late from a cancelled/old runtime so they cannot attach to the next attempt.

Run the relevant established checks for each milestone, not all repository suites after every documentation/UI edit:

- Web: `npm test`, `npm run lint`, `npm run build`.
- API: formatting, `go vet ./...`, `go test ./...`, and targeted race tests for ingestion/projection if concurrent state changes.
- Phone: `npm run typecheck`, `npm test`, `npm run test:native`; validate JS bundling and packaging. Native changes also require real Swift C-ABI/link checks and a rebuilt signed Release app.
- Pheme when changed: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`, plus relevant offline packaging checks. Record any existing unrelated failure separately; do not represent it as a green run.

Use a disposable server database for automated ingestion fixtures. Do not mix fixture observations into the real phone dataset. Update `docs/api/metrics.md`, `docs/api/benchmark-ui.md`, `docs/benchmark-methodology.md`, `ios-app/NATIVE_TESTING.md`, and AGENTS implementation status only as milestones actually ship.

Physical acceptance uses the actual phone/build and records its hardware and OS. Start with a Release build containing two device-loadable ASR models and two reasoning models, three conversations of two turns each, and two repetitions:

```text
4 configurations x 3 conversations x 2 repetitions
= 24 planned conversation executions
= 48 measured turns and 96 stage attempts on the all-success path
```

Warm-up/loading attempts are additional and excluded from those totals; internal ASR retry events do not change the 96 stage-attempt count. This small matrix verifies the pipeline, not research-grade p95 or model superiority.

Acceptance scenarios:

1. Normal matrix: verify all planned executions, exact model/configuration identities, ordered turns and native context isolation. Match selected phone-run timings to their exported events.
2. Offline matrix subset: disable network after preparation, execute and inspect locally, restart the app, then sync later. Server receipt requires no additional inference and produces no duplicate records.
3. Interrupted conversation: background/cancel and separately terminate/relaunch during inference. Account for the interrupted execution; rerun from conversation start with a new ID, preserving old evidence.
4. Collection pressure: simulate archive/full-queue/storage failures and delayed drains; confirm new work pauses, protected records remain, and any loss/truncation is visible.
5. Model failure: use a missing/incompatible artifact in a controlled case; confirm explicit failure/skips and no fallback to another configuration.
6. Attribution: validate two installations and runtime counter restarts through fixtures, then verify actual device/app/build fields in phone exports and the server snapshot.
7. Resource gate: verify supported providers on the device, explicit missing sensors, categorical thermal states and collector overhead. Record charging/display/sync conditions.
8. Reconciliation: local archive, server snapshot, overall dashboard, conversation details and JSON/CSV/Markdown agree on jobs, executions, outcomes, series counts and completeness for the same filters/watermark. Repeat upload acknowledgments leave totals unchanged.
9. Overall visibility: start at Overall Metrics, watch new results and metric categories appear, filter by configuration/date/device, and open contributing conversations without losing the overview filters. Verify totals exceed the visible activity-page size when appropriate. During offline testing, the phone advances while the web displays only received data with its freshness; after sync the matching snapshots reconcile.

Release acceptance requires complete attribution for every admitted operation, explicit accounting for every planned job, no duplicate observations under retries, and no false complete-data labels. Report measured values only for providers actually implemented and verified. Keep historical reconstruction and new explicit conversation data distinguishable.
