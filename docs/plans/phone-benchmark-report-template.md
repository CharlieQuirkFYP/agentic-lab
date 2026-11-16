**Phone benchmark report — template**

Status: **EMPTY TEMPLATE — no benchmark measurements are contained in this document.** Template prepared on 2026-11-10 for the [implementation plan](phone-conversation-benchmarks-plan.md). Planned counts below describe the acceptance workload, not completed work. The autonomous runner and shared report generator are implemented; actual run outcomes belong in the generated evidence reports and the physical acceptance record.

Populate a copy from a consistent phone archive or server snapshot after implementation and testing. Retain this empty template in documentation; real recordings and generated report artifacts belong in configured ignored/external storage.

**1. Report identity and measurement coverage**

| Field | Value |
| --- | --- |
| Report ID / revision / generated time | Pending |
| Scope | Overall filtered history / selected sweep / selected conversation — choose at generation |
| Selected filters and time zone | Pending |
| Snapshot source | Phone local archive / server received records — choose at generation |
| Snapshot watermark / input hash | Pending |
| Collection period and retained coverage | Pending |
| Execution status | Not run |
| Telemetry completeness | Not assessed |
| Device / installation / iOS | Not recorded |
| App build / code revision / native runtime builds | Not recorded |
| Dataset ID / revision / protocol version | Not recorded |
| Model configuration manifest(s) | Not recorded |
| Upload policy / last receipt / synchronization state | Not recorded |

Execution completion and telemetry completeness are independent. A completed inference can have missing samples; an interrupted attempt can have fully accounted partial observations. The web snapshot contains only received data. Any remotely reported queue state has its own observation time. A filtered or retained-history export is not labelled lifetime history.

**2. Test protocol and validation evidence**

Before performance measurements, verify collection/grouping with local fake engines and a disposable Go database. Record test commands, source revision, actual outcomes and evidence locations separately from phone measurements. A green unit/integration test is not physical-phone performance evidence.

| Verification layer | What it establishes | Observed result / evidence |
| --- | --- | --- |
| Grouping and statistics fixtures | Correct identities, unavailable values, sample counts, filters and deterministic summaries | Not run |
| Phone persistence/native lifecycle tests | Correct association before inference, cancellation settlement and recovery bookkeeping | Not run |
| HTTP ingestion tests | Atomic storage, late/out-of-order records, duplicate acknowledgments and conflicts | Not run |
| Dashboard/export tests | Full filtered totals across pages; consistent overview/detail/export snapshots | Not run |
| Release build / actual Swift-Rust bridge | Production linkage and packaged model identity | Not run |
| Physical-device acceptance | Local inference and collection under the scenarios below | Not run |

Prepare fixed recordings grouped into three two-turn conversation cases. Suggested case purposes are an initial report with added detail, an initial report followed by a correction, and a report containing local terminology/numbers with a follow-up. These are workload descriptions; they do not establish correctness of the eventual reply or an implemented incident-confirmation workflow. Optional reference transcripts stay local unless explicitly exported.

Record each utterance once for this dataset revision; replay identical bytes for each configuration. Keep independent cases separate and retain context only between turns of the same case. Freeze artifact checksums, prompts, effective settings and protocol before the run.

| Protocol field | Planned rule / value to capture |
| --- | --- |
| Inference location | Physical phone, verified with network disabled after model setup |
| Workload | Fixed audio script; benchmark submits actual ASR output to reasoning |
| Concurrency | One admitted native operation at a time |
| Model configuration | ASR/reasoning hashes, quantization, threads, language, prompt/role hash, context/output limits and decoding parameters |
| Warm-up and model loading | Recorded separately; excluded from measured conversation statistics |
| Ordering | Reproducible randomized blocks; save seed and actual order |
| Cooldown | Save configured policy and actual waits; thermal state unknown until a provider is verified |
| Device conditions | Charging state, starting battery level, power mode, brightness and ambient conditions if known |
| Sampling | Record enabled providers, interval, actual coverage and overhead characterization |
| Upload interference | Record policy; defer uploads during measured operations by default |
| Failure/rerun handling | Keep failures; stop dependent turns; rerun whole interrupted conversations under new IDs |

First acceptance matrix: **2 ASR models x 2 reasoning models x 3 conversations x 2 repetitions = 24 planned conversation jobs**. With two turns per case, the all-success path has 48 measured turns and 96 native stage attempts. Warm-ups/loading are additional. Native internal ASR retries are diagnostics within a stage attempt. Execution reruns are additional records tied to the original job.

The first matrix tests the pipeline. It yields six successful conversations and twelve observations per stage per configuration at most on the all-success path. Under the plan's minimum of 20 eligible observations, per-configuration p95 is **insufficient samples**, not a populated percentile. A larger performance campaign uses a separately frozen repetition count after the pilot; it does not pool different configurations to increase percentile sample size.

**3. Overall metrics — work represented so far**

Populate these from the entire selected snapshot, independently of how many rows the dashboard displays. For an overall export, include all matching manual chats and sweeps in retained coverage; for a sweep export, use the selected sweep only. Planned workload is shown only when that scope has a manifest.

| Work measure | Planned for first acceptance sweep | Observed in selected snapshot |
| --- | --- | --- |
| Configurations | 4 | Not recorded |
| Unique conversation jobs | 24 | Not recorded |
| Initial conversation executions | 24 on all-success path | Not recorded |
| Additional execution reruns | No automatic reruns | Not recorded |
| Measured turns | 48 on all-success path | Not recorded |
| Native stage attempts | 96 on all-success path | Not recorded |
| Successful / failed / cancelled / interrupted executions | Outcomes to be measured | Not recorded |
| Pending / running / skipped jobs | Outcomes to be measured | Not recorded |
| Warm-up / loading operations | Recorded separately | Not recorded |
| Partial / unlinked / truncated observations | Coverage to be measured | Not assessed |
| Dropped native events / missing durable records | Coverage to be measured | Not assessed |

Show each available metric group, with one row for each compatible device/configuration/stage/name/unit/scope/source combination. Unknown configuration in legacy observations remains labelled unknown.

| Metric group | Latest / timestamp | Mean | Min / max | Valid count | Unavailable count / reason |
| --- | --- | --- | --- | --- | --- |
| Transcription operation duration, ms | Not measured | Not measured | Not measured | Not recorded | Not assessed |
| Reasoning operation duration, ms | Not measured | Not measured | Not measured | Not recorded | Not assessed |
| Active conversation inference duration, ms | Not measured | Not measured | Not measured | Not recorded | Not assessed |
| Native diagnostic stage timings, per stage | Not measured | Not measured | Not measured | Not recorded | Not assessed |
| Process CPU and process memory, per provider | Not measured | Not measured | Not measured | Not recorded | Provider implementation/verification required |
| Other observed numeric metrics, separate rows | Not measured | Not measured | Not measured | Not recorded | Not assessed |

Use distinct status rows for battery level/charging and categorical thermal state. Battery changes are device-level observations over a defined interval; thermal categories are not numeric temperatures. Whole-device power, energy and GPU utilization remain unavailable unless a verified provider exists at the time of execution. Do not use zero for missing readings.

Include activity-over-time counts, separate per-configuration latency trends and provider-specific resource charts. Gaps remain visible. Show recent conversations, active work and data freshness. These charts refer to the same filters and snapshot as the tables.

**4. Model-configuration comparison**

Replace A/B with exact artifact identities; configuration manifests accompany the short labels. Keep device/build/dataset/protocol groups separate.

| Configuration | ASR | Reasoning | Successful / started executions | ASR median, ms (n) | Reasoning median, ms (n) | Active conversation median, ms (n) | Collection coverage |
| --- | --- | --- | --- | --- | --- | --- | --- |
| C1 | ASR A — pending selection | LLM A — pending selection | Not measured | Not measured | Not measured | Not measured | Not assessed |
| C2 | ASR A — pending selection | LLM B — pending selection | Not measured | Not measured | Not measured | Not measured | Not assessed |
| C3 | ASR B — pending selection | LLM A — pending selection | Not measured | Not measured | Not measured | Not measured | Not assessed |
| C4 | ASR B — pending selection | LLM B — pending selection | Not measured | Not measured | Not measured | Not measured | Not assessed |

Expand each configuration with per-case statistics, numeric counts, mean/min/max, all failure outcomes, retries and measurement availability. Latency uses one explicitly bounded measurement per eligible operation. Success-only latency tables state that denominator; failed/interrupted elapsed times remain available separately. Means are weighted by their contributing observations rather than averages of unweighted case means.

Sampled peak memory and timestamp-aware CPU statistics appear only after their native providers pass verification. Do not mix loading, warm-up, human review, playback or uploading with active inference latency. Do not sum nested core stage timings into an operation that already includes them.

Findings: **Pending measurements.** Identify the fastest measured configurations for this dataset and conditions only after comparable observations exist. Transcript/reply quality and incident-task success are not established by execution completion. No overall model winner or battery-endurance claim is inferred from this acceptance sweep.

**5. Conversation and attempt drill-down**

Repeat the following block for each execution. Include incomplete executions and link reruns to their original planned job.

| Conversation field | Value |
| --- | --- |
| Conversation / case / repetition / planned job | Pending |
| Configuration / device / runtime session | Pending |
| Started / finished / active time / wall time | Not measured |
| Execution outcome / reason code | Not run |
| Telemetry completeness / finalization | Not assessed |
| Prior execution / rerun relationship | Not recorded |

| Turn | Stage | Attempt / native run ID | Outcome | Operation duration, ms | Valid/unavailable samples | Completeness |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Transcription | Pending | Not run | Not measured | Not recorded | Not assessed |
| 1 | Reasoning | Pending | Not run | Not measured | Not recorded | Not assessed |
| 2 | Transcription | Pending | Not run | Not measured | Not recorded | Not assessed |
| 2 | Reasoning | Pending | Not run | Not measured | Not recorded | Not assessed |

Attach grouped metric summaries and raw event references to each attempt. Numeric values, statuses and unavailable reasons remain separate. Metrics-only exports use neutral case labels and structured error codes; spoken content, replies and freeform errors remain outside this report.

**6. Reliability and measurement validation**

Run these as separately identified acceptance scenarios, not hidden failures inside the normal performance sample. Each scenario needs its own run IDs, expected outcome, observed outcome and evidence reference. Use explicit check states (`not run`, `pass`, `fail`, `blocked`), and identify the evidence method (fixture, simulator/build, or physical phone) and build/device where applicable. A fixture pass never becomes a physical-device pass.

| Scenario | Expected evidence | Actual outcome |
| --- | --- | --- |
| Offline inference | Phone generates replies and local metrics without network/server inference | Not run |
| Offline collection and later sync | Local overview advances; server advances after receipt; matching snapshots reconcile | Not run |
| Retry after lost acknowledgement | Server stores one logical copy of the same observation | Not run |
| Cancellation / backgrounding | Active operation settles; scheduling pauses; ownership and outcomes are retained | Not run |
| Process termination / relaunch | Interrupted execution is visible; rerun has new identity and starts with fresh context | Not run |
| Storage pressure / queue full | New operations pause; protected datasets and unsynced observations remain | Not run |
| Native observer overflow / collection gap | Missing data is visibly accounted for; report cannot be labelled complete | Not run |
| Missing/incompatible model | Explicit configuration failure and skipped jobs; no silent fallback | Not run |
| Multiple installations / runtime counter reuse | IDs remain isolated; no cross-device merges | Not run |
| Overview with more than one page | Headlines match full filtered data, not visible page length | Not run |
| Stale response / reconnect | Older snapshots do not replace newer state; freshness remains visible | Not run |
| Resource provider / collector overhead | Correct phone process/device scope, cadence, gaps and overhead measurements | Not run |

Use deterministic fault injection for disk/queue/overflow cases; distinguish those fixtures from a physical-device test. Device tests use the actual signed Release app. Where available, compare native resource readings with a tool observing that same phone and document scope/cadence differences. Treat simulator readiness as build evidence only.

Overall Metrics acceptance also checks a snapshot containing both manual chats and sweeps, unknown legacy metadata, every observed metric family, valid zeroes and unavailable readings. Add a newly observed metric during the test and verify it becomes discoverable. Date/device/configuration filters must persist across drill-down and apply equally to cards, charts and exports. The complete filtered totals must cover more than 100 batches independently of visible activity pages; before offline data syncs, the local and server overviews display their different coverage honestly.

**7. Report generation and reconciliation record**

The planned generator runs this sequence:

1. Choose Overall Metrics filters, a sweep or a conversation. Capture a phone SQLite snapshot or a server ingestion watermark; freeze that input for this report revision.
2. Read all matching records across pages, preserving installation, conversation, attempt and batch identities. Verify exact-identity retries, parent associations, finalization record ranges, drop counts and missing data. Conflicting records are reported as errors rather than arbitrarily selected.
3. Build the conversation hierarchy and normalize metric series using the shared reporting module. Separate legacy reconstructed observations, providers, scopes, configurations and protocol versions.
4. Compute statistics at the stated attempt/turn/conversation unit, including valid counts and explicit failures. Produce chart series with unavailable intervals left as gaps.
5. Write the readable Markdown report, attributed CSV tables and raw/reproducible JSON bundle. Record the input watermark/hash, filters, protocol and generator version in all formats.
6. Compare overview, detail and export counts/summaries for the same snapshot. Advance the report to verified only when the relevant checks pass; retain partial-data labels for incomplete telemetry. Later arrivals require a new revision.

| Reconciliation check | Actual result |
| --- | --- |
| Every planned job has an explicit state | Not assessed |
| All started executions and additional reruns are accounted for | Not assessed |
| Every admitted stage attempt has an unambiguous parent | Not assessed |
| Finalized record ranges / sample counts reconcile | Not assessed |
| Phone and server match after the relevant records are acknowledged | Not assessed |
| Overview, conversation detail and JSON/CSV/Markdown match at the same snapshot | Not assessed |
| Repeated uploads do not change unique counts | Not assessed |
| Statistical sample units and denominators are documented | Not assessed |
| Unsupported measurements remain unavailable with reasons | Not assessed |

Final report disposition: **NOT RUN.** No model ranking, measured latency, resource finding or test-pass claim is available in this template.
