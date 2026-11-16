# Phone results and graphs plan

Status: implemented in the working tree, 2026-11-13. Results now includes overall
and individual metric graphs, turn/operation detail, scoped saved history,
compatible sweep comparisons and snapshot-based sharing. Automated and native
build checks pass. See the [validation record](../testing/phone-results-acceptance-2026-11-13.md)
for executed checks and remaining physical interaction/performance acceptance.
The sections below retain the design and acceptance requirements.

See the [three-screen visual draft](phone-results-mockup/phone-results-draft.png)
or its [scalable browser view](phone-results-mockup/index.html). All values in the
mockup are illustrative. It shows overall results, an individual metric graph,
and the same metric narrowed to one operation.

The objective is to let KLASS users understand a phone conversation or benchmark
without reading raw telemetry: how long processing took, which stage took the
time, whether the operations completed, and what happened to the phone during
the run. Keep Chat as the primary experience and make Results easy to reach.

## Recommended first delivery

Add a Results entry using the existing `/metrics` route. Preserve the TUI-style
hierarchy explicitly: **overall results and graphs → individual metric graph →
turn/operation evidence**. The overall view contains summary metrics and compact
graphs for the selected conversation or benchmark configuration; every measured
metric can open its own detail graph. Ship per-turn transcription/reply bars,
app-memory history, and a thermal-state timeline first. CPU and other recorded
metrics remain reachable through the metric list in that first delivery. Follow
with benchmark model comparisons and a compact shareable summary. Existing
JSON/CSV/Markdown exports remain available.

Use plain labels such as **Speech processing**, **Reply generation**, **Processing
time**, **App memory**, and **Completed turns**. Each chart needs units, the number
of contributing executions/readings, and a short explanation. Technical names,
model hashes, source/scope and raw references remain available in detail.

## What the TUI actually has

The current Telemetry navigation is **Overview / Metrics / Conversations / Logs**.
The internal enum still calls the third tab `Runs`; its screen prefers saved
conversations and retains the earlier single-run report as a fallback.

| Implemented TUI feature | Evidence | Phone adaptation |
| --- | --- | --- |
| Current model/state, transcription stage durations and latest resource readings | `pheme-va/crates/cli/src/tui/ui.rs`, `draw_overview` | Small Results cards with familiar labels and phone-supported measurements. |
| Searchable, categorized live metric table with Now/Min/Max/Avg/N | `ui.rs`, `draw_metrics`; `telemetry.rs`, `MetricSeries` | A searchable Details view; select a metric to open its history. |
| Live metric plots with elapsed-time axes and scope/source/unit labels | `ui.rs`, `draw_metric_detail` | Native time-series charts; single values become cards/points rather than fabricated trends. |
| Conversation list, overview, saved chat, stage runs and per-run details | `chat_ui.rs`, `draw_conversations` | Conversation → turn → operation navigation using phone archive identities. |
| Conversation-wide or run-specific metric summaries, numeric sample plots and retained raw events | `chat.rs`, `chat_metric_summaries`; `chat_ui.rs`, `Detail::Metric` | Tap a graph to inspect contributing operations/readings. The TUI conversation plot uses sample index; the phone should use an explicit time axis for resource history. |
| Conversation totals for completed/failed/cancelled turns, audio, transcription, reasoning, title overhead, review and wall time | `chat_ui.rs`, `overview` | Reuse the distinctions; display only timing boundaries actually recorded on the phone. |
| Historical single-run reports and metric plots; bounded saved archives | `ui.rs`, `draw_run_report` / `draw_historical_detail`; `history.rs` | Retained local result history, with clear coverage and separate live/historical views. |

The TUI currently offers generic numeric sample plots and detailed tables. The
proposed stacked stage bars, thermal bands and business-facing model comparisons
are new presentation work. A multi-panel business dashboard is not already
implemented in the TUI. Its desktop resource providers, title timing and review
timing must not be treated as available phone measurements automatically.

Source: [TUI telemetry screens](../../pheme-va/crates/cli/src/tui/ui.rs),
[conversation screens](../../pheme-va/crates/cli/src/tui/chat_ui.rs),
[series definitions](../../pheme-va/crates/cli/src/tui/telemetry.rs).

## Baseline foundations and gaps when this plan was drafted

| Area | Current implementation | Consequence for this plan |
| --- | --- | --- |
| Phone metrics | `ios-app/app/metrics.tsx` shows counts, current/history conversations, turn/attempt timings, compatible metric groups, delivery controls, pin/removal and three export formats. It has no charts. | Evolve the existing route and extract reusable native components. |
| Phone benchmark | `ios-app/app/benchmark.tsx` records/imports a dataset, configures a sweep, shows progress, and exports results. | Keep workload setup here; link a finished or running sweep to scoped Results. |
| Shared calculations | `shared/phone-benchmarks/` supplies revision resolution, attribution, completeness, duration groups and reports in platform-independent TypeScript. | Reuse these semantics and add pure presentation selectors, with shared fixtures. |
| Web graphs | `PhoneMetrics.tsx` has numeric sample timelines; `PhoneComparisonCharts.tsx` compares compatible configurations; `PhoneSnapshotAnalysis.tsx` uses complete report snapshots for execution distributions. | Reuse data transformations where practical; browser SVG/DOM components require native rendering. |
| On-phone collection | `ResourceSampler.swift` samples at 500 ms; the root collector drains in the foreground. Attempt IDs connect readings to conversations and turns. | Existing observations support the main charts without a new inference path or collector. |
| Archive performance | `benchmarkStore.snapshot()` synchronously reads the full archive; `/metrics` projects it on each store version. The archive allows up to 50,000 records / 48 MiB. `liveSummary()` already demonstrates a smaller query. | Bounded local reads and refreshes are part of the first delivery, before live graphs. |

Relevant code: [phone metrics](../../ios-app/app/metrics.tsx),
[benchmark screen](../../ios-app/app/benchmark.tsx),
[local store](../../ios-app/src/benchmarkStore.ts),
[collector](../../ios-app/src/benchmarkCollector.ts),
[shared report contract](../../shared/phone-benchmarks/README.md),
[native resource sampler](../../ios-app/modules/pheme-native/ios/ResourceSampler.swift).

## Phone experience

**Results overview**

The first screen shows the current/latest conversation or selected benchmark,
its model names and date, and a small set of meaningful values. Manual chat and
benchmark results have separate selectors so their workloads are not pooled.
Default to the latest result; history is a paginated list with date and model
filters. The visible scope applies to cards, charts, drill-down and export.

- Manual chat: completed/failed/cancelled turns, typical recorded processing time
  when enough eligible turn data exists, and sampled peak app memory if available.
- Benchmark: completed versus planned jobs, separate failure/interruption counts,
  and stage processing summaries for the selected configuration.
- A compact line identifies local saved data, last update, and pending upload
  status. Move server address and retry controls into Sync details.
- Missing required timings or incomplete collection produce a visible coverage
  message. Unsupported optional sensors do not generate empty cards.

The overview includes graphs as well as totals: processing across turns and
compact memory/thermal histories for the selected result. These are the overall
graphs across that result's contributing turns/operations, not only the latest
operation. Keep units on separate panels; seconds, MiB and thermal categories
must not share one numeric axis. Resource panels share an elapsed-time interval
where timestamps permit it; processing bars use turn/execution labels. A metric
list below the overview exposes all recorded metric groups, including CPU and
battery when available. Tapping a metric opens its dedicated graph with the same
conversation/configuration scope retained.

**Conversation detail**

```text
< Results                  Conversation / date
Speech model + reply model
Completed turns       Recorded processing time

Processing by turn                         seconds
Turn 1  [ Speech processing ][ Reply generation ]
Turn 2  [ Speech processing ][ Reply generation ]
        Tap a stage for its duration and outcome

App memory                                  MiB
[ line across elapsed time; gaps stay visible ]

Thermal state
[ labelled categorical bands across the same interval ]

Details                 Share summary
```

This conversation overview is the overall graph level. Its metric cards, graph
panels and metric-list rows link to the individual metric level below.

This is a proposed layout, not a plot of measured results. On narrow screens,
cards stack vertically. Tapping a bar or sample reveals its value, timestamp or
elapsed position, turn, stage and status. Provide an accessible text/table
equivalent, large text support and labels that do not depend on colour alone.

**Individual metric detail**

Show a larger graph for the selected metric over the whole selected conversation
or configuration, with its unit, coverage and contributing sample/execution count.
For compatible numeric readings, include latest/min/max/average, as in the TUI;
categorical measurements show labelled states instead. A single reading is a
point/value, not an invented trend. Preserve gaps between disjoint operations.
Allow drill-down to one turn or operation and back to the overall metric graph;
show the active scope clearly. Raw readings and source/scope metadata sit below
the graph. The metric graph is part of the first delivery, not deferred with
model comparisons or sharing polish.

**Benchmark comparisons**

Open a sweep through View results. Use horizontal bars for median transcription
and reasoning operation times by model combination, alongside completion counts
and sample counts. Add sampled peak app-memory comparisons after the basic
latency view. Display loading and warmup separately. A configuration that failed
before a conversation started still appears through its planned jobs.

Only compare compatible device/installation, OS/app/build, dataset, protocol,
mode, stage, timer boundary and metric source/scope/unit. Retain model artifact
and effective-setting identities even when the display uses short model names.
Check recorded charging, power-mode and thermal conditions; differences remain
visible and prevent an unqualified ranking. Manual chats with different content
are history, not controlled model comparisons. Prefer factual captions such as
“Reply generation accounted for most recorded processing time” when supported by
the selected data. An overall recommended model needs quality evidence too.

## Chart and metric rules

| Visual | Data and calculation | Display rule |
| --- | --- | --- |
| Per-turn stacked bars | Latest distinct transcription and reasoning attempt durations under the turn, including retries as explicit attempts | Sum sequential native-call durations; keep cancelled/failed attempts identifiable. Do not add nested runtime timings again. Text input has no transcription segment. |
| Processing-time trend | One eligible turn or attempt value per plotted execution, with a fixed unit of observation | Plot within a compatible configuration. Never use resource samples as extra executions or sum stage medians into a total median. |
| Memory line | `resident_memory_bytes`; optional separate `physical_footprint_bytes` series | Label as whole-app process memory. Display MiB/GiB conversion consistently. Max means highest sampled reading, not a continuous peak or model-only allocation. |
| CPU line in Details | `cpu_percent`, `percent_one_core`, `darwin.task_info`, process scope | Retain the one-core basis; values can exceed 100%. Explain that 100% represents one fully used core. |
| Thermal band | `thermal_state`: nominal, fair, serious, critical | Categorical labels, not a temperature in degrees. Preserve unknown intervals. |
| Battery detail | Device battery level and charging state over a selected interval | Show recorded level/percentage-point change and charging context. Do not infer watts, joules or hours of endurance. |
| Completion summary | Latest explicit turn outcomes for manual chat; job outcomes and planned job count for sweeps | Keep attempts/retries separate. Execution completion does not establish incident correctness, report saving or dispatch. |
| Model comparison | Compatible execution-duration distributions and job outcome counts | Show median and n first. p95 requires at least 20 eligible successful, complete executions in that group. |

Manual turns and conversations currently do not persist aggregate `duration_ms`;
their attempts do. Sweep turns/conversations already record a sum of measured
native calls. For old and new manual history, expose a derived **recorded
processing time** with its contributing attempt IDs and completeness. If an
expected attempt is absent, active or has no duration, show the subtotal as
partial rather than a complete turn time. Derivations must not rewrite the raw
records or enter the existing recorded-duration distributions silently. Any new
derived distributions need an explicit measurement definition shared by the UI
and report. Human review time, idle time, audio playback and upload time are not
speech-to-audible-reply latency.

Manual New chat currently closes the conversation with status `succeeded` even
when child turns were cancelled. Present its recorded lifecycle status separately
from per-turn outcomes; use completed-turn counts for the plain-language card.
Viewing Results must not finish/reset a conversation or change inference state.

Raw metric groups deliberately do not publish p95. The current phone metric row
still prints a p95 placeholder for every numeric group; replace that with
execution-only percentile presentation. Median/mean resource readings are sample
statistics, not time-weighted measurements.

Availability has three visible cases: measured (including valid zero), required
but missing/partial, and optional unsupported. Keep GPU utilization, power and
energy in an expandable capability explanation while no provider exists. Quality
and incident-task success stay unscored until reference data and scoring exist.
These remain research requirements where applicable; hiding an empty card does
not satisfy the measurement requirement.

## Implementation structure

```text
Existing native sampler + root collector
                  |
          Local SQLite archive
                  |
   Scoped read queries + snapshot identity
                  |
     Shared projection/presentation selectors
                  |
       Native Results screens and charts
                  |
       Snapshot-consistent report exports
```

Extend `benchmarkStore.ts` with filtered summary/history reads and a scoped
snapshot read that includes required ancestors/finalization evidence. Capture a
durable sequence watermark plus a local change generation so later arrivals or
history removal cannot mix revisions across one displayed result. A pagination
cursor affects visible history only; totals cover the whole selected retained
scope. Counts must never be described as lifetime totals after removal.

Add `shared/phone-benchmarks/presentation.ts` (proposed) for metric labels, chart
series, compatible comparison groups, derived processing totals and availability
policy. Preserve raw record references, units, scopes, sample counts and outcome
denominators. Avoid importing `web/src` into Expo. Keep the canonical report
format compatible; add any necessary derived definitions additively.

Add a focus-aware `usePhoneResults` hook and native components under
`ios-app/src/components/results/` (proposed). Coalesce visible live updates at
most once per second; do not rebuild the full archive on every write. Read a
bounded recent series for live detail and lazy-load historical detail. Keep one
collector in `_layout.tsx`; UI subscribers never call native poll/drain or start
another sampler. Pause rendering work when the route is unfocused/backgrounded.
Resource sampling is configurable and may have been disabled for a run; consult
its recorded settings and actual samples instead of assuming every conversation
has CPU/memory history. Keep local transcript/reply content out of metric exports.

Use plain React Native views for bars/cards and `react-native-svg`, installed
through Expo at implementation time, for line charts and thermal bands. The
current package has no chart renderer. This is one narrowly justified dependency;
no general dashboard/chart framework is needed. Verify Expo compatibility and
native linking before selecting the version. Use a proposed display budget of
200 points per series, preserving gap boundaries and per-bucket extrema; raw
statistics and exports always use all selected observations.

Heavy historical analysis and complete exports should be on demand after native
work settles. A frozen full report clones/projects its input and is unsuitable
for a per-frame/per-write live data source. Profile the implementation on the
phone with a large archive before choosing caches or additional database indexes.

## Delivery sequence and ownership

| Change | Scope and owner | Dependency / acceptance | Complexity |
| --- | --- | --- | --- |
| 1. Results data contract | Data owner: store queries, shared selectors and fixtures | Freeze timer/denominator/availability rules. Full-scope totals agree with reports; histories page correctly; scoped snapshots preserve attribution and finalization. | Medium |
| 2. Results UI and conversation graphs | UI owner: `/metrics`, new detail route, hook, chart components; primary owns navigation integration | UI can start against fixed fixtures after the contract is agreed. First delivery works offline and provides overall metrics/graphs plus individual metric graphs, with conversation → turn/operation drill-down, without affecting chat. | Medium |
| 3. Sweep comparisons and sharing | Comparison owner: pure compatible-group selectors; UI owner: comparison route and benchmark links | Reuse changes 1–2. All planned configurations, failures and sample denominators appear. Share summary and existing exports use the same snapshot and filters. | Medium |
| 4. Physical acceptance and refinement | Primary integrator: large-archive checks, device profiling, accessibility and docs | Verify the complete journey and collection overhead before accepting the feature. Add a screenshot/PDF summary only if required after the initial shareable text/report view. | Medium |

Run the data and UI work concurrently once their input/output types are agreed;
give the UI fixed fixtures initially. Keep one owner for shared selectors and one
for route components to avoid conflicting edits. Swift/Rust sampling, Go ingestion
and incident workflow changes are not expected for the initial charts.

## Acceptance and required checks

- Deterministic selectors: correct stage sums and retries; missing/zero values;
  manual derived versus recorded sweep durations; cancelled child turns;
  configuration failures with zero started conversations; incompatible groups;
  duplicates, revisions and p95 thresholds; categorical thermal data and gaps.
- Store tests: more than one page of history, complete filtered totals, stable
  snapshots, parent/finalization references, late writes, acknowledged-history
  removal, and preservation of pending/pinned records.
- Chart interaction: zero/one/many samples, long gaps, extrema-preserving
  downsampling, no misleading connecting lines between executions, readable units,
  large text, VoiceOver and colour-independent outcomes.
- Navigation parity: open overall metrics/graphs, select any recorded metric,
  inspect its full-result graph and statistics, narrow to a turn/operation, then
  return with the original result scope preserved. Check current and saved data.
- Offline physical phone: finish a manual multi-turn chat; open Results; inspect
  a stage and memory/thermal history; relaunch and reopen the same result; share
  it without a server. Repeat with sampling disabled and an interrupted turn.
- Sweep acceptance: inspect a mixed-success sweep; confirm failed configurations
  remain visible; compare compatible models; reconcile on-screen values and
  JSON/CSV/Markdown for one snapshot; sync later without duplicate counts.
- Performance: test a fixture archive near the configured record/byte limits;
  record screen-open/render times, JS stalls, process memory and dropped events.
  On the phone, compare Results visible/hidden under the same fixed workload in
  counterbalanced repeated runs. Record conditions and variance before claiming
  a rendering overhead percentage. All collector/drop changes need investigation.

For implementation, run `npm run typecheck` and `npm test` from `ios-app/`,
`node --test shared/phone-benchmarks/tests/*.test.ts` from the repository root,
and the iOS Expo export used by CI. Run the existing Swift/native contract and
link checks when the renderer dependency/native project changes. If shared
selectors or exports change, also run affected web fixture tests, lint and build.
Physical Release-device acceptance remains required; fixture/simulator passes do
not establish on-phone responsiveness or measurement quality.

The current [physical acceptance record](../testing/phone-benchmark-acceptance-2026-11-11.md)
verifies prior collection/reporting work. It also records incomplete active-load
calibration, unvalidated energy measurements and uncontrolled comparison
conditions. Those limitations still apply when the same data is shown as graphs.
