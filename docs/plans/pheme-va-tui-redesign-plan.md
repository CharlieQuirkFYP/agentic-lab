# Pheme VA TUI Redesign Plan

## Status

The core redesign is implemented in `pheme-va/crates/cli/src/tui/`: the workspace has `Overview`, `Metrics`, `Runs`, and `Logs` tabs, separate live and historical telemetry contexts, worker-side model preparation/switching, bounded logs, search/highlighting, and persistent bounded run history. This document records that baseline and the remaining presentation/testing enhancements.

## Purpose

This document defines the current and next version of the Pheme VA terminal test bench. It keeps the TUI focused on local speech-model evaluation while making the distinction between live monitoring and historical run analysis explicit.

The TUI is a development voice console and local benchmark-inspection surface. It is not a production desktop hotkey application and does not provide partial live transcription. A completed recording still produces one final transcription result.

The existing Rust, Ratatui, `metrics`, and Pheme VA worker architecture is retained. The redesign does not add a monitoring daemon, database server, distributed queue, or new UI framework.

## Core navigation model

The telemetry workspace has four tabs:

```text
[1] Overview    [2] Metrics    [3] Runs    [4] Logs
```

| Tab        | Meaning                                                                  | Data context                                 |
| ---------- | ------------------------------------------------------------------------ | -------------------------------------------- |
| `Overview` | Current model, current request, transcript, and compact resource summary | Live/current state                           |
| `Metrics`  | Continuously updating metric table and live metric graphs                | Live state; active run shown as context only |
| `Runs`     | Historical run selector and after-action reports                         | One selected run after pressing `Enter`      |
| `Logs`     | Structured application, worker, build, and native diagnostics            | Live and retained log history                |

The current navigation uses `Runs` instead of the earlier `Graphs` tab. Historical run selection belongs only to `Runs`; it does not silently filter the live `Metrics` tab.

In a terminal, “click” means selecting an item with the keyboard and pressing `Enter`.

## Live versus historical data

### Live Metrics

`Metrics` is a small system monitor for the current TUI process and the active speech request.

- It is not pinned to the last completed run.
- It continues updating while the TUI is idle.
- When a request is active, the current run ID is shown as context.
- While a request is active, running statistics are calculated for that request.
- Pressing `Enter` on a metric opens a live graph that continues receiving samples.
- When no request is active, the header says `LIVE / idle` and the table shows the latest host measurements plus a short rolling history.

Example headers:

```text
LIVE METRICS / idle
LIVE METRICS / run-0008 BUSY
LIVE METRICS / run-0008 COMPLETE
```

The run label is informational. It must not cause the live screen to display an old run merely because that run was selected previously.

### Runs

`Runs` is a historical run browser.

```text
[1] Overview    [2] Metrics    [3] Runs    [4] Logs

┌ COMPLETED RUNS ──────────────────────────────────────────────────────────┐
│ > run-0008   live microphone   speech       16.55 s   14:20:31          │
│   run-0007   recording.wav     speech       12.42 s   14:17:08          │
│   run-0006   live microphone   empty        16.10 s   14:12:44          │
└─────────────────────────────────────────────────────────────────────────┘

[j/k] select run  [Enter] open report  [Esc] back
```

- `j/k` or arrow keys select a run.
- `Enter` opens the selected run’s after-action report.
- `Esc` returns to the run list or the previous TUI screen.
- An active run may appear as `BUSY`; its report can update until completion.
- A completed run is frozen and remains available for later inspection.
- `[` and `]` may cycle runs inside the run browser/report, but they do not change live Metrics context.

## Runtime data model

The telemetry store retains two related but separate views of data.

### Live sample store

The live store contains a bounded rolling window of samples from the current TUI session. It is used by `Overview`, `Metrics`, and live metric details.

Each metric series is identified by all of the following (the current implementation stores `category` as a derived string):

```rust
struct SeriesKey {
    name: String,
    category: String,
    scope: MetricScope,
    source: String,
    unit: MetricUnit,
}
```

The source and scope are part of the identity so that, for example, process CPU and system CPU cannot be merged into one graph accidentally.

Each live series exposes:

- Latest value
- Minimum
- Maximum
- Average
- Sample count
- First and last timestamps
- Unavailable sample count
- Last unavailable reason, when applicable
- Numeric samples for graphing

The live store is bounded by a maximum event count so an idle TUI cannot grow memory indefinitely. The current store retains up to 10,000 events.

### Historical run store

Each request gets a run record containing its report metadata and samples captured while that request was active. The current persisted shape is:

```rust
struct RunReport {
    run_id: String,
    model_id: String,
    model_family: String,
    backend: String,
    runtime: Option<String>,
    revision: Option<String>,
    source: String,
    status: String,
    transcript: String,
    raw_transcript: String,
    language: Option<String>,
    audio_duration_seconds: f32,
    gate_decision: String,
    segment_count: usize,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    error: Option<String>,
    events: Vec<MetricEvent>,
}
```

The run record also owns or references the metric events associated with its `run_id`. The historical report is not reconstructed from whichever run happens to be selected in a global telemetry field.

When a run is selected in `Runs`, report values and historical metric detail use that run's saved events only.

## Continuous resource monitoring

The resource sampler runs continuously while the TUI is open, not only while a model is decoding.

The request lifecycle is:

```text
TUI starts
    ↓
Create live rolling telemetry stream
    ↓
Recording starts → create run context
    ↓
Sample resources during microphone capture
    ↓
Recording stops
    ↓
Continue sampling during normalisation, gating, transcription, cleanup, and result handling
    ↓
Finalize RunReport and freeze its run samples
    ↓
Return to live idle monitoring
```

The run context must be created before microphone recording begins. All samples belonging to the request must share the same run ID, including recording, normalisation, transcription, cleanup, and final result handling.

The implemented resource sampling cadence is approximately 250 ms. The sampling loop runs separately and does not block the UI event loop or the inference worker.

The active Bench and Processing screens should include a compact monitor strip:

```text
MODEL whisper-large-v3-turbo | RUN run-0008 | BUSY

CPU 42.1%   RAM 1.77 GB   GPU 37.0%   TEMP 68°C   BATTERY -2.0%

Recording: 08.62s       Transcription: 16.43s
```

The strip and the Metrics tab must read from the same latest-sample store. They must not maintain separate resource measurements.

### Resource measurement rules

Values must remain explicitly unavailable when the host cannot measure them. Missing measurements must never be replaced with zero.

The Linux desktop provider may expose:

- Process CPU percentage
- System CPU percentage
- Process RAM bytes
- System RAM bytes
- DRM GPU utilisation when `gpu_busy_percent` is available
- Hottest finite component temperature reported by available sensors
- Signed single-battery percentage-point change from the first valid reading

These values require clear labels:

- Process CPU may exceed 100% because it can sum usage over multiple cores.
- DRM GPU utilisation is the busiest readable GPU card, not process-specific GPU usage.
- Component temperature is not ambient or whole-device temperature.
- Battery drain is a coarse signed capacity change; a negative value can indicate charging.
- Battery-terminal power is not automatically whole-device power.
- CPU/GPU component power is not automatically whole-device power.

Whole-device power, joules, and watt-hours remain unavailable unless a provider with a verified measurement boundary is supplied. An unavailable metric should display its reason in the table and report, but should not be plotted as a numeric zero.

## Overview screen

Overview should be a compact, framed dashboard rather than a long unstructured metric list.

```text
┌ PHEME VA / OVERVIEW ────────────────────────────────────────────────────┐
│ ACTIVE MODEL                                                            │
│ whisper-large-v3-turbo / whispercpp                                    │
│                                                                         │
│ CURRENT RUN                                                             │
│ run-0008     live microphone     speech                               │
│                                                                         │
│ TRANSCRIPT                                                              │
│ The recognised transcript appears here in a dedicated panel.           │
│                                                                         │
│ PERFORMANCE                                                             │
│ Audio duration       8.62 s                                             │
│ Normalisation       119 ms                                             │
│ Transcription     16433 ms                                             │
│ End-to-end        16552 ms                                             │
│                                                                         │
│ LIVE RESOURCES                                                          │
│ CPU 42.1%   RAM 1.77 GB   GPU 37.0%   Temp 68°C                       │
└─────────────────────────────────────────────────────────────────────────┘
```

Overview should prioritise:

1. Active model and backend
2. Current request state
3. Latest transcript and raw transcript access
4. Audio and processing timings
5. Latest live resource values
6. Clear empty, no-speech, and error states

Detailed metric history belongs in Metrics and Runs, not in the Overview body.

The screen should remain usable at the minimum supported terminal size of 80×24. Long transcript and error content should scroll inside its own framed area instead of pushing other panels off-screen.

## Metrics screen

Metrics should look like a category-organised table, not a graph page and not a raw event dump. The table must not include a timestamp column in its primary layout.

```text
[1] Overview    [2] Metrics    [3] Runs    [4] Logs

┌ LIVE METRICS / run-0008 BUSY ────────────────────────────────────────────┐
│ Category / Metric                         Now       Min       Max   Avg  │
│                                                                         │
│ > RESOURCES                                                            │
│   Process CPU                            42.1 %    0.8 %   455.7 % 92.4 │
│   System CPU                             24.4 %    2.3 %    26.3 % 14.1 │
│   Process RAM                             1.77 GB   1.72 GB  1.80 GB    │
│   System RAM                             17.73 GB  17.26 GB 17.80 GB    │
│   GPU utilisation                         37.0 %    0.0 %    61.0 % 24.3 │
│   Temperature                             68.0 °C   55.0 °C  85.0 °C 69.4 │
│                                                                         │
│ > POWER / ENERGY                                                        │
│   Battery drain                           -2.0 %    -2.0 %   -2.0 % -2.0 │
│   Whole-device power                     unavailable                    │
│   Energy                                 unavailable                    │
│                                                                         │
│ > TIMING                                                               │
│   Normalisation                           119 ms    119 ms   119 ms  119 │
│   Transcription                         16.43 s   16.43 s  16.43 s16.43 │
│   End-to-end                             16.55 s   16.55 s  16.55 s16.55 │
└─────────────────────────────────────────────────────────────────────────┘

[j/k] select  [Enter] live graph  [1-4] tab  [/] search  [x] clear  [Esc] back
```

### Table behaviour

- Categories are visible section rows such as `RESOURCES`, `POWER / ENERGY`, `TIMING`, `COUNTS`, and `STATUS`.
- Categories and metric rows have a selected state.
- `j/k` and arrow keys move through categories and metrics.
- The selected row is highlighted, including its category and metric name.
- `Enter` on a category opens a category graph page.
- `Enter` on a metric opens that metric’s live graph.
- The table updates in place as new samples arrive.
- The latest value is visually emphasised while min/max/average remain readable.
- Unavailable rows remain visible and explain why no value is available.
- Metrics with different units must not be combined into one axis.
- The table can scroll without changing the live data context.

The category summary is calculated from live samples while idle and from the active run’s samples during a request. It does not silently switch to a previously selected completed run.

## Live metric graph screen

Selecting a metric from Metrics opens a dedicated live metric-detail screen. The screen continues receiving samples while open.

```text
┌ LIVE / GPU UTILISATION ─────────────────────────────────────────────────┐
│ Run: run-0008 BUSY       Source: sysinfo       Scope: Device            │
│ Current: 37.0%   Min: 0.0%   Max: 61.0%   Avg: 24.3%   Samples: 42      │
│                                                                         │
│ 60% ┤                         ●                                         │
│     │                   ●          ●                                    │
│ 30% ┤          ●                                                     │
│     │    ●                                                             │
│  0% ┼────────────────────────────────────────────────────────────────  │
│     0s                 5s                 10s                 16s       │
└─────────────────────────────────────────────────────────────────────────┘

[Esc] back  [j/k] metric  [/] search  [x] clear
```

The graph must include:

- Metric name, category, scope, source, and unit
- Current, minimum, maximum, average, and sample count
- A relative time axis
- Gaps for unavailable samples
- No interpolation across unknown data
- A visible live indicator while samples continue arriving
- A selected/highlighted metric identity that remains selected when returning to Metrics

A category graph may show multiple compatible metrics together. Metrics with incompatible units should be split into separate panels or axes.

## Runs screen and after-action report

The Runs tab contains the list first. It does not immediately show a graph for whichever run happens to be first.

```text
┌ RUNS ────────────────────────────────────────────────────────────────────┐
│ > run-0008   BUSY       live microphone   whisper-large-v3-turbo        │
│   run-0007   complete   recording.wav     whisper-large-v3-turbo        │
│   run-0006   empty      live microphone   whisper-large-v3-turbo        │
└─────────────────────────────────────────────────────────────────────────┘

[j/k] select run  [Enter] open report  [/] search  [Esc] back
```

Pressing `Enter` opens the selected run's report. The current report includes metadata, transcript/raw transcript access, and a run-only metric snapshot. Pressing `Enter` on a snapshot metric opens its historical numeric detail when enough samples exist. The richer multi-panel graph grid below remains a presentation enhancement, not the current renderer.

The intended report layout is:

```text
┌ RUN-0007 / AFTER-ACTION REPORT ─────────────────────────────────────────┐
│ Model: whisper-large-v3-turbo / whispercpp                             │
│ Source: live microphone       Status: speech                          │
│ Language: en                  Audio: 8.62 s                            │
│ End-to-end: 16.55 s           Segments: 4                              │
│                                                                         │
│ TRANSCRIPT                                                              │
│ The final recognised transcript appears here.                          │
│                                                                         │
│ RAW TRANSCRIPT                                                          │
│ Original backend output appears here.                                  │
└─────────────────────────────────────────────────────────────────────────┘

┌ RESOURCES ───────────────────────────┐ ┌ TIMING ────────────────────────┐
│ CPU / RAM / GPU / temperature        │ │ Normalisation / gate / decode  │
│       ╭──╮                           │ │       ╭────╮                    │
│   ╭───╯  ╰──╮                        │ │   ╭───╯    ╰──╮                 │
│ ──╯         ╰────────                │ │ ──╯           ╰────             │
└──────────────────────────────────────┘ └─────────────────────────────────┘

┌ POWER / ENERGY ─────────────────────┐ ┌ STATUS / COUNTS ───────────────┐
│ Battery drain                         │ │ Speech gate: SpeechDetected    │
│ Whole-device power: unavailable       │ │ Transcript: Speech              │
│ Energy: unavailable                   │ │ Recovery attempts: 0            │
└──────────────────────────────────────┘ └─────────────────────────────────┘
```

The after-action report should contain:

- Selected run ID and timestamps
- Model ID, family, backend, runtime, and revision where available
- Input source
- Final status and error state
- Final transcript
- Raw transcript
- Detected/configured language
- Audio duration
- Gate decision
- Segment count
- Stage and end-to-end timings
- Resource, timing, power/energy, status, and count graph panels

The graph grid should resemble a compact monitoring dashboard:

- One panel per useful category where practical
- Multiple lines when metrics have compatible units
- A legend for each series
- The selected metric drawn brightly
- Other matching series visible but dimmed
- Unavailable series shown in the legend with a reason and omitted from numeric plotting
- Responsive fallback to a vertical stack at smaller terminal sizes
- Scrolling when the report exceeds the terminal height

A completed report is historical and does not update. A `BUSY` report may update until the worker publishes its final result.

## Logs screen

Logs remain structured and bounded. The Logs tab should include:

- Timestamp
- Level
- Component
- Message
- Optional run ID or request ID where useful

Components include `manifest`, `worker`, `model-loader`, `adapter-build`, `metrics`, `native-stderr`, and `recorder`.

On Unix, Whisper/GGML, ALSA, and Cargo/native diagnostics are captured into the log store rather than being written directly over the Ratatui screen. Unix stderr capture remains active during model loading, recording, retries, adapter builds, and worker shutdown. Other platforms currently use the fallback path and do not provide this native capture.

The log store should keep a bounded number of entries and report dropped entries rather than blocking the UI.

## Universal search and highlighting

The filter is a search tool shared by all telemetry tabs, not a Logs-only filter and not a destructive list filter.

```text
[/] Search: gpu_
```

Rows remain in their normal category/order so the user retains context. Matching text is highlighted in the row, report, legend, or log message. Nonmatching content may be dimmed but should not disappear by default.

Example:

```text
RESOURCES

  Process CPU
  System CPU
> GPU utilisation
  Temperature
```

The `gpu_` or matching substring should use a strong accent/reversed style wherever it occurs:

```text
> [gpu_] utilisation
```

Search applies to:

- Overview labels and transcript text
- Metrics categories, metric names, units, scopes, sources, and values
- Live graph titles and legends
- Runs, including run IDs, model IDs, source, status, and report text
- Historical graph legends and report data
- Logs, including component and message

Suggested controls:

```text
[/] or Ctrl+F    start search
[Enter]          keep search and return to navigation
[Esc]            leave search mode without clearing the query
[Backspace]      delete a character
[x]              clear search
[n]              next match
[N]              previous match
```

The search state should be shared by the telemetry workspace, while each screen maintains its own scroll/selection position. Selecting a metric or run must not clear the query. A match count and current match position should be visible when search is active.

For graphs:

- Matching series and legend text are bright.
- Nonmatching series are dimmed but remain visible.
- The selected metric is brighter than all other series.
- Search does not turn unavailable samples into plotted points.

## Model selection and automatic adapter builds

Model selection remains manifest-driven. The catalog should distinguish:

- Unsupported model family
- Adapter not compiled
- Required artifact missing
- Ready to load

Selecting a supported model whose adapter is not compiled automatically starts a background Cargo build. The user does not need to restart manually with `--features whisper` or `--features zipformer`; the TUI restores the terminal and restarts into the validated adapter cache after success.

The build behaviour is:

```text
Select uncompiled model
    ↓
Show BUILDING ADAPTER screen
    ↓
Run Cargo asynchronously with the requested adapter feature
    ↓
Build into the isolated `target/tui-adapters/build/` directory and publish a validated executable under the fingerprinted `target/tui-adapters/cache-v2/` entry
    ↓
Capture Cargo/compiler diagnostics in Logs
    ↓
Restore the terminal and stop workers safely
    ↓
Restart the TUI using the rebuilt executable
```

Build requirements and constraints:

- The original source checkout and Cargo/Rust toolchain must be available.
- Native build dependencies may be required.
- Cargo may download build dependencies and the LiteRT runtime.
- Selecting a known supported model may download its pinned, checksum-verified artifacts automatically before compilation; unsupported/custom artifacts remain manual.
- The build uses an isolated target directory and does not overwrite the running executable.
- Already-enabled adapter and acceleration features should be preserved.
- Only the known adapter families may be requested; no arbitrary shell command may be constructed from a manifest value.
- `Esc` cancels the build without blocking the UI.
- Build failure returns to the picker and leaves the current model active.
- Successful restart retains TUI settings and saved run reports but resets in-memory live metrics, logs, and retry audio. This reset is stated on the preparation screen.

The build screen should show:

```text
BUILDING ADAPTER

Building adapter for zipformer-small...

Cargo is running in the background.
The first build may take several minutes.

[t] compiler logs   [Esc] cancel build   [q] quit
```

## Error, silence, and retry states

The TUI must distinguish:

- No microphone/device available
- Microphone stream failure
- Silence detected by the gate
- Insufficient speech
- Speech detected but empty backend transcript
- Known silence marker
- Dictionary prompt echo
- Backend/model failure
- Adapter build failure

The result view should show both status and useful context. For example, `SpeechDetected` means the gate admitted audio; it does not guarantee that Whisper produced text. An empty backend result should say so explicitly rather than displaying a blank transcript panel.

Retries must:

- Use a new request ID and run ID
- Avoid processing duplicate worker events
- Avoid saving duplicate results
- Keep native diagnostics inside Logs
- Preserve the current model if retry fails

## Key map

```text
Global:
  q                 quit
  ?                 help
  t                 open telemetry workspace
  f or /            search, where applicable
  Ctrl+F            search

Overview / Metrics / Runs / Logs:
  1-4               switch telemetry tab
  Esc               leave current detail view/workspace
  j/k or arrows     move selection or scroll
  n/N               next/previous search match
  x                 clear search

Metrics:
  Enter             open live graph for category/metric

Runs:
  Enter             open selected run report
  [ / ]             previous/next run

Logs:
  c                 clear retained logs
```

The footer must change based on the active screen and must not claim that `[3]` opens a generic graph page after the tab is renamed to Runs.

## Rust and Ratatui implementation notes

The implementation is already organised within the existing TUI modules:

```text
pheme-va/crates/cli/src/tui/app.rs
pheme-va/crates/cli/src/tui/ui.rs
pheme-va/crates/cli/src/tui/telemetry.rs
pheme-va/crates/cli/src/tui/worker.rs
pheme-va/crates/cli/src/tui/events.rs
pheme-va/crates/cli/src/tui/logs.rs
pheme-va/crates/metrics/src/resources.rs
```

The current implementation already:

- uses `TelemetryTab::Runs` rather than the old `Graphs` tab;
- provides a metric detail/chart view from `Metrics`;
- keeps live metric selection separate from historical run selection;
- identifies series by metric name, category, scope, source, and unit;
- stores run reports independently from the live event window;
- retains run metadata alongside metric events;
- highlights search matches in Ratatui text; and
- reuses `Chart`, `Dataset`, `Table`, `Paragraph`, `Block`, and `Tabs` widgets.

Remaining UI work is primarily richer responsive layouts and a full multi-panel historical graph grid. No new dependency is required. Native-log capture on Unix, automatic allowlisted model download and adapter build/restart, model language handling, and resource-provider improvements should be retained.

## Testing and validation

Tests are kept beside the packages they cover and use deterministic local fixtures. The existing suite covers the main reducer, telemetry, history, rendering, and resource-provider behavior; extend it as the remaining presentation work lands.

Telemetry tests should cover:

- Live samples remain visible while idle.
- Active run samples use one run ID from recording through completion.
- Historical run selection does not alter live Metrics.
- Metric identity separates name, category, scope, source, and unit.
- Latest/min/max/average/sample count calculations.
- Fractional and negative values.
- Unavailable samples and reasons.
- Missing samples create graph gaps rather than zeroes.
- Bounded live and historical retention.
- Selection recovery after new samples, filtering, and eviction.

Application tests should cover:

- `Metrics` opens live metric detail with `Enter`.
- `Runs` opens the selected report with `Enter`.
- `[3]` is labelled and behaves as Runs.
- Returning from a report preserves the selected run.
- Search matches and highlights text across all four tabs.
- Search does not remove nonmatching context by default.
- `n/N` moves between matches.
- Build cancellation and failure leave the active model usable.
- Retry IDs prevent duplicate result handling.

Rendering tests using Ratatui’s `TestBackend` should cover at least 80×24 and a larger terminal for:

- Framed Overview layout
- Category Metrics table
- Selected-row highlighting
- Live graph and unavailable graph states
- Runs list
- After-action report and graph grid
- Logs and highlighted search matches
- Long transcripts, errors, and unavailable reasons

Resource tests should use synthetic sysfs fixtures and verify:

- GPU peak-card selection
- Temperature handling
- Battery baseline, signed changes, gaps, and multiple-battery safety
- Whole-device power/energy remain unavailable without a verified provider

Run the standard Rust checks from `pheme-va/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Model-backed tests remain explicitly ignored unless model and audio paths are supplied through environment variables. Model weights, recordings, and generated benchmark artifacts must not be committed.

## Acceptance status

The following redesign criteria are implemented:

- `[2] Metrics` is a live monitor and is not pinned to a historical run.
- The active run ID is shown as context without acting as a hidden filter.
- Metrics are displayed in category sections without a timestamp column in the main table.
- Selecting a metric and pressing `Enter` opens live metric detail; selecting a run and pressing `Enter` opens its report.
- `[3] Runs` first displays a run list.
- Historical reports remain tied to their selected run and do not change when live samples arrive.
- Resource sampling covers idle, model loading/switching, recording, and processing when enabled.
- The Overview is framed and compact, and search works across the telemetry workspace.
- Search highlights matching text instead of simply hiding context.
- Unix native Whisper/ALSA/Cargo diagnostics are captured without overwriting the TUI.
- Selecting an uncompiled adapter starts background preparation and automatic restart without a manual feature-flag restart.
- Unsupported measurements remain explicitly unavailable with a reason; component values are not presented as whole-device power.
- The TUI remains responsive while recording, sampling, inference, searching, and preparing adapters.

Remaining acceptance work is a richer responsive multi-panel historical graph grid, broader non-Unix native-diagnostic capture, and any additional rendering tests required by those enhancements. Formatting, Clippy, workspace tests, and deterministic rendering tests should pass after changes.

## Boundaries

This plan does not introduce:

- Production incident workflow state or report persistence
- A separate monitoring service
- PostgreSQL, Redis, Kafka, Docker, or Kubernetes
- Cloud inference
- Unrestricted or unverified model-weight downloads; automatic downloads are allowlisted and checksum-verified.
- Partial streaming transcription
- Unverified whole-device power estimates
- Mobile UI implementation

The Pheme VA core remains authoritative for audio normalisation, transcription status, transcript guards, and model adapter boundaries. The TUI presents those results and coordinates local development workflows without duplicating core inference rules.
