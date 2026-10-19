# Pheme VA

Local, backend-first speech and reply engine for Agentic Lab. Audio becomes a reviewed transcript; a separate instruction model answers approved text. Web/TUI hosts own microphone capture and speech playback; portable core owns neither terminal UI nor mobile navigation.

```text
native microphone / web upload
            |
            v
     core
       - WAV/PCM normalization
       - mono 16 kHz conversion
       - energy/silence gate
       - dictionary prompt construction
       - backend-neutral transcription options and decoder guards
       - conservative transcript guards
       - optional cleanup adapter
            |
            v
     raw + processed transcript
```

The workspace contains these crates:

| Crate          | Purpose                                                        |
| -------------- | -------------------------------------------------------------- |
| `core`         | Platform-independent pipeline and adapter traits               |
| `va_runtime`   | Shared conversation orchestration, inference gate and events    |
| `metrics`      | Per-run pub/sub metrics, resource samplers, and batch contract |
| `cli`          | WAV client and small terminal microphone recorder              |
| `server`       | Development HTTP wrapper around the same core                  |
| `ffi`          | Small C ABI for iOS/Android hosts (`include/pheme_va.h`)       |
| `whispercpp`   | `whisper.cpp` model adapter                                    |
| `zipformer`    | Optional LiteRT Zipformer CTC model adapter                    |
| `reply-model`  | Persistent stdio conversation adapter; no native GGML linkage  |
| `reply-native` | llama.cpp adapter and host-owned `pheme-reply-worker` binary |

Model weights are not committed. The local `models/` directory contains the checksum manifest; run `./scripts/download-model.sh --list` to see the explicitly selectable artifacts. The no-argument command downloads and verifies the supported Whisper baseline.

## What is implemented

- WAV input with 8/16/24/32-bit integer and 32-bit float support.
- Channel downmixing and a portable windowed-sinc resampler to 16 kHz.
- OpenWhispr-inspired energy gate with explicit `silence`, `insufficient_speech`, and `speech` states.
- Bounded, deduplicated dictionary/snippet prompts.
- Whisper decoder defaults based on the inspected OpenWhispr settings: blank/non-speech suppression, zero initial temperature, entropy/log-probability thresholds, and no-context decoding for independent clips. These are options for compatible backends, not a dependency on OpenWhispr.
- Raw transcript preservation.
- Known silence-marker filtering, conservative dictionary-prompt echo detection, and one bounded retry without the prompt before discarding an echo.
- Cleanup as a swappable `TextCleaner` trait, with `LlmTextCleaner` wrapping a replaceable `LanguageModel` trait. The default formatter only normalizes whitespace/capitalization; it does not invent facts.
- An in-process `whisper-rs` adapter in the separate `whispercpp` crate behind the `whisper` feature. The model is loaded once and reused for each recording.
- A model-agnostic `Transcriber` trait in `core`, separate `whispercpp` and optional LiteRT `zipformer` model crates, and manifest-based CLI model selection. SeaLLMs-Audio remains download-only and is not part of the active runtime.
- A WAV fixture test that exercises the complete audio path without model weights.
- An ignored real-model transcription test for a supplied speech recording.
- A ratatui-based local TUI with remembered manifest model selection, worker-owned model switching, WAV browsing, live microphone capture, local voice/typed Chat, metrics graphs, and structured logs.
- An HTTP host with stateless WAV transcription and web-owned reviewed turns, streamed replies, cancellation/reset, recovery/inspection, and isolated reply tests.
- A separate local GGUF reply runtime with native cancellation, fresh per-call KV state, and a registry-bound incident role. Qwen2.5-1.5B-Instruct Q4_K_M is the starter model, not a validated deployment choice.
- A TUI workspace with alphabetic Chat/Web/Tests/Models/Telemetry navigation, turn-bound transcript editing, one manifest-backed Models page, inline local downloads, automatic model-selection persistence, reply role choices, combined prompt preview/hash, and remembered automatic voice replies with playback status.
- A Go-delegated React voice console with complete-turn WAV capture, explicit review, real reply streaming, and local-voice-only browser speech synthesis.
- A portable `metrics` crate with synchronous pub/sub, per-run filtering, application timings/status events, resource snapshots, explicit unavailable values, and batched export data.
- Linux/macOS/Windows process/system CPU and RAM sampling through `sysinfo`, with extension points for native mobile sensors and device power/thermal providers.
- Core and incident-analysis instrumentation that keeps pure text analysis separate from transcription.
- A C ABI bridge suitable for a thin Swift/Kotlin host, including JSON metric-batch draining.

Default tests do not download/load weights. Workspace builds compile the native reply worker, requiring CMake, a C/C++ compiler and libclang for bindgen. On Debian/Ubuntu: `sudo apt-get install libasound2-dev cmake clang libclang-dev`. Client and server inference tests use local fakes.

## Verify the core

From `pheme-va/`:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Use Whisper locally

New Whisper downloads go to `models/transcript/whisper/ggml-large-v3-turbo.bin`. Existing legacy weights are not moved automatically; see [path migration](models/README.md#local-registry-and-path-migration) before downloading again. Run `./scripts/download-model.sh` to reproduce the setup; its checksum and source are recorded in [`models/README.md`](models/README.md). `large-v3-turbo` is an initial CPU baseline, not a validated mobile configuration; compare it against the LiteRT Zipformer adapter on the target device.

Build and transcribe an existing WAV file:

```bash
./scripts/download-model.sh
cargo run --release -p cli --features whisper -- \
  --stt-model whisper-large-v3-turbo \
  transcribe recording.wav \
  --language en \
  --dictionary KLASS,whisper.cpp,"west entrance"
```

The WAV may have any supported sample rate/channel count. It is normalized inside the core before the selected speech model receives it.

### Local TUI chat and test bench

On Linux, install the system audio development package required by `cpal` if it is missing, for example `libasound2-dev` on Debian/Ubuntu. Start the TUI from `pheme-va`:

```bash
cargo build --release -p cli -p reply-native --features cli/whisper
./target/release/cli tui
```

The TUI opens Models and restores saved selections automatically. The header shows the selected voice and reply models and their readiness; the list marks both with **[SELECTED]**, independently of the browsing cursor. The WAV browser defaults to `samples/` when launched from `pheme-va` (`pheme-va/samples/` when launched from the repository root). Use `--audio-directory` to override it or `tui --reconfigure` to reopen setup.

On Models, **Enter** prepares and selects the highlighted model and remembers the choice automatically. Missing files first request download confirmation. Speech adapters load directly, use a cached build or compile as needed. A different reply selection verifies its artifacts and role, then automatically relaunches the TUI to activate it. Selecting an already active model avoids reloading it. **b** opens Chat once both selected models are ready. Finish the conversation before switching reply models. Model details distinguish compiled adapters, validated cached builds and adapters needing preparation.

A cached build is reused without invoking Cargo: lookup prefers the exact adapter combination, then a compatible build containing all required adapters and accelerators. Cache misses build in the background, preserving already-enabled adapters, then restart automatically. Every cached generation includes a verified `pheme-reply-worker` beside the CLI. Older generations remain on disk; changed build inputs or incompatible metadata require rebuilding. Samples, roles and documentation do not invalidate these binaries. On Unix, the preparation screen streams actual Cargo/native output and automatically follows the newest lines. Press `t` for full logs or Escape to cancel; failure leaves the current model available. Settings and saved run reports survive exits and backend restarts. Live metrics, logs, and retry audio remain in memory only.

Automatic setup requires the original source checkout, Cargo/Rust, and native build dependencies. Known Whisper/Zipformer model IDs can download missing pinned, checksum-verified artifacts through `scripts/download-model.sh`. Cargo may also download dependencies or the LiteRT runtime. Backend cache generations live under `target/tui-adapters/cache-v2/` with version 4 metadata; versions 2/3 are retained but require a fresh build. Unix builds reuse a locked `target/tui-adapters/build/` for incremental compilation. Neither overwrites the running executable. This is a development convenience, not runtime compilation for mobile deployments.

On some macOS Command Line Tools installations, the Whisper adapter build fails with `fatal error: 'mutex' file not found` or another missing C++ standard header. The TUI may only show `Cargo adapter build failed (exit status: 101)`; press `t` during preparation to see the native logs. If the logs show this header error, point the compiler at the headers in the macOS SDK and retry from `pheme-va/`:

```bash
export CPLUS_INCLUDE_PATH="$(xcrun --show-sdk-path)/usr/include/c++/v1"
cargo run --release -p cli --features whisper -- tui
```

The export applies only to the current shell. Other macOS installations may build without it.

To avoid the initial build/restart, optionally compile both adapters up front:

```bash
cargo run --release -p cli --features 'whisper,zipformer' -- \
  --model-manifest models/manifest.toml \
  --stt-model whisper-large-v3-turbo \
  tui
```

You can also download Zipformer artifacts manually:

```bash
./scripts/download-model.sh zipformer-small
```

After selecting both models on Models, **b** opens Chat with the existing microphone/WAV Source panel. After transcription the draft appears under **EDIT YOUR MESSAGE / NOT SENT**: **Enter** sends it, **e** opens editing, **Alt+Enter** adds a newline, and **Esc** returns to review while retaining edits. **i** starts a typed message. **c** finishes the current conversation and starts another; **d** finishes it. **x** discards/cancels the current turn. All shortcut rows start at the left with fixed spacing and wrap whole items. Chat and Tests call the shared `va_runtime` library directly; no Go or HTTP service is needed. Typed messages skip transcription. The **MESSAGE** composer stays visible, and **i** can draft the next message during generation; sending waits for the current inference to settle.

**m** opens Models, **t** opens Telemetry, **b** returns to Chat, and **w** opens Web inspection. Only inside Telemetry does **1–4** select Overview, Metrics, Conversations, or Logs. Conversations opens a list first; Enter opens aggregate stats, **r** lists individual stage runs, and **h** shows saved chat. Enter on a metric opens its raw samples and graph. Missing readings remain unavailable. See [chat controls, screen examples and retained measurements](../docs/tui-chat.md).

On Unix, native stderr (including Whisper/ALSA diagnostics) is captured into bounded logs; capture is not yet implemented on other platforms.

#### Saved run history and privacy

Conversation snapshots and legacy run reports persist locally, including chat messages, reviewed wording, processed/raw transcripts, source filenames, model/runtime/revision metadata, timestamps, errors, and raw metric events (including unavailable reasons). **This can contain sensitive speech and paths.** No recordings or audio buffers are written to history. The backend-specific `TranscriptionResult` is not restored; historical report display uses the saved metadata and events.

History is a versioned JSON snapshot at `$XDG_STATE_HOME/pheme-va/tui-history.json` (absolute XDG path), falling back to `$HOME/.local/state/pheme-va/tui-history.json`. This is bounded development-console history, not the planned SQLite incident/session database; it reuses the existing JSON dependency rather than adding a database runtime. Keep one TUI session per history path. On Unix, newly created state directories use mode `0700` and snapshot files `0600`; this is not encryption. Existing parent directory permissions are not changed.

Version 2 reads legacy version 1 files. History retains at most 100 stage runs/legacy reports and 10,000 raw events per run; oldest closed conversations are evicted together and active conversations are protected. Each conversation accepts at most 32 turns. Aggregates retain counts/min/mean/max even when old raw events are dropped. Restored conversations are read-only archives; restarting does not resume runtime context. Snapshots have a 64 MiB size ceiling; an oversized save reports an error and retains the previous snapshot. Writes run on a background thread, coalesce pending snapshots, and use file sync plus atomic replacement. Completed/failed reports are queued on the next TUI tick; shutdown drains worker results and flushes history before any automatic restart. Forced termination can still lose an unflushed update. Malformed, oversized, or unsupported history prevents startup without overwriting the file: move it aside for inspection or explicitly remove it to start fresh. Write/clear failures appear in status and logs, and a failed shutdown flush prevents automatic restart.

In Conversations, press `c`, then `y` to delete **all** saved conversations and reports (not only filtered reports); Escape cancels. Clearing is disabled while a request or recording is active. Reports disappear only after deletion succeeds; other input is paused while deletion is in flight. This does not delete source WAVs, memory-only retry audio, or external backups, and is not secure erasure.

Resource readings include process/system CPU and RAM, available component temperatures, and Linux DRM GPU utilization and single-battery capacity drain. Process CPU can exceed 100% because it sums core utilization. GPU is the busiest readable card, not per-process use; temperature is the hottest reported component, not ambient temperature. Battery drain is a signed percentage-point decrease since the sampler's first valid reading (negative while charging), with coarse sensor resolution. Whole-device power and energy remain unavailable without a verified provider; component or battery-terminal power is not silently relabeled.

The current desktop sampler deliberately does not implement a whole-device power provider: it returns unavailable even if Linux exposes RAPL or battery `power_now`. RAPL measures CPU package/core domains, while battery power measures battery-terminal charge/discharge (particularly misleading as total consumption while plugged in). Joules and watt-hours are integrated from verified whole-device watts, so they are unavailable for the same reason. Enabling these metrics requires a calibrated power provider with a documented measurement boundary, not elevated permissions or a model change.

Current manifest models return a final transcript after a complete clip; they do not provide partial live words. This is intentionally a local development and model-evaluation console, not the production UI or a global desktop hotkey implementation.

### Shared web/TUI voice loop

From `pheme-va/`, build the HTTP host and reply worker, then start with an explicit transcription/reply pair:

```bash
cargo build --release -p server -p reply-native --features server/whisper
./target/release/server \
  --stt-model whisper-large-v3-turbo \
  --reply-model qwen2.5-1.5b-instruct-q4-k-m \
  --metrics-enabled
```

Explicit catalog choices download missing pinned files directly through `scripts/download-model.sh`. The Whisper and reply downloads are approximately 1.62 GB and 1.12 GB respectively. Alternatively download in the TUI Models view first. Startup validates the incident prompt and loads/prewarms the selected runtimes once. The normal reply dependency has **no feature flag**; `server/whisper` is the existing optional STT build choice. For Zipformer use `server/zipformer` and its catalog ID.

`pheme-reply-worker` must be beside the server executable (or set `PHEME_VA_REPLY_WORKER` to its trusted path). This host-owned stdio child isolates conflicting Whisper/llama.cpp GGML libraries; it is not a second HTTP server. See [runtime/model details](models/README.md#qwen-native-reply-baseline). A failed worker requires its host to restart; CPU/RAM accounting must include that child.

Start Go from `api/` in another terminal (`API_CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 go run ./cmd/server`); it delegates voice calls to `http://127.0.0.1:8000` by default through `PHEME_VA_URL`. Start the web from `web/` (`npm ci && npm run dev`), then open `/voice`. Vite proxies the public Go API. Inspect the same server with:

```bash
cargo run --release -p cli -- tui --server-url http://127.0.0.1:8080
```

- **b Chat:** reviewed voice/WAV/typed turns with conversation context, model typing indicators and streamed replies. **Enter** confirms/sends, **e** edits, **c** starts a new conversation, **d** finishes, and **x** cancels/discards.
- **w Web:** observe the web conversation; **e** edits its pending transcript and **Enter** approves. The answer still streams to the web. Inspection is silent; **p** is deliberate local replay.
- **s Tests** from Chat: isolated microphone/WAV STT or typed/reviewed reply tests, without conversation history. **Enter** submits reviewed text. These use the local runtime and its compute gate, with independent test context.
- **m Models:** local manifest catalog, downloads and remembered selections; see [Models controls](#unified-models-page).
- **t Telemetry:** Overview/Metrics/Conversations/Logs keep **1–4** while open; **Esc** returns. Minimum size is 80×24. **z** stops local playback; **q** quits the client and cancels its own pending console turn.

Chat uses two left-aligned footer rows with consistent four-space gaps: navigation/conversation controls, then actions for the current state. Mic and WAV stay visible while idle. Replay appears for a completed reply and becomes Stop voice during playback. **? Help** lists Web, Tests, next WAV, retry and scrolling shortcuts; their keys still work.

The same grouped list/details Models page serves standalone and connected modes. **Voice & transcription** (`purpose = "transcript"`) and **Reasoning & reply** (`purpose = "reply"`) come from the manifest, not purpose tabs or a second picker. Legacy Whisper/Zipformer entries without `purpose` remain transcription entries. Local Chat uses the saved reply selection at TUI startup. Each host owns one persistent engine, reply adapter and shared inference gate. Artifact presence is shown separately from runtime readiness.

Press **v** in Chat to toggle **Voice replies ON/OFF**; the setting is remembered and starts off for older configs. When on, each new completed reply is spoken automatically. The top-right Chat indicator is bright green when on, dim when off, cyan while speaking and yellow if speech fails. Turning it off stops current speech; **z** stops just the current playback, and **p** optionally replays the last reply. Desktop speech uses locally installed `espeak-ng`, falling back to `espeak`; synthesis errors preserve displayed replies. Chat and isolated Tests have separate automatic playback settings. Web only chooses browser-reported local voices and has no remote fallback—verify actual offline behavior on your browser/OS.

`--server-url` without a value uses Go on port 8080 for Web inspection/approval only; Chat and Tests stay local with or without it. Separate server and TUI processes do not share loaded weights or sessions. Remembered TUI selections live in its XDG config. The server reads the legacy `server_stt_model`/`server_reply_model` settings unless explicit model flags override each slot; local voice selection updates `selected_stt_model`. `--startup-choices <file>` supplies dedicated TOML choices, including optional ordered `reply_prompt_files`. Model-ID/path flags conflict rather than silently override one another. See [role composition and startup precedence](#reply-roles-and-startup-precedence); an active server must restart to apply model/prompt changes.

Defaults: one active inference, one pending web turn, six successful history pairs, 32 retained turn results, 300-second review, 120-second reply generation, 180-second STT deadline. Failed/cancelled replies are not successful history. WAV/STT is complete-turn, not partial streaming; only reply text streams, with TTS after completion. The STT adapter has no native cancellation boundary yet: cancelled/timed-out STT keeps compute reserved until it settles. Restart loses web history; explicit Reset clears web history, not isolated tests. Retry protection retains at most 256 lifetime keys; a full ledger rejects new keys rather than duplicate inference.

These are trusted development conversations with in-memory runtime context and read-only local archives. Confirmed incident reports and persistent resumable sessions remain planned. Both listeners default to loopback; network exposure needs HTTPS, explicit allowed origins and external authentication/access controls. Neither the web nor any HTTP readiness/inference endpoint downloads/selects models. See [voice API](../docs/api/voice.md) and [cross-service tests](../tests/README.md).

### Unified Models page

Open `m` in either mode. The header always shows the two selected models and their readiness. The left side retains grouped manifest rows for both purposes and marks the selections; the right side shows the highlighted model's metadata and preparation state. Browsing does not change either selection.

| Key                | Action                                                                                                                                                                                                                                                 |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `/`                | Search across all manifest entries.                                                                                                                                                                                                                    |
| Arrows / `j` / `k` | Select a model row.                                                                                                                                                                                                                                    |
| PgUp / PgDn        | Scroll model details.                                                                                                                                                                                                                                  |
| Enter              | Prepare and select the model, remembering it automatically. Missing files request download confirmation. A different reply model activates through automatic TUI relaunch. |
| `b`                | Open Chat when both selected models are ready. |
| `p`                | Preview the reply row's combined role text and exact-byte SHA-256.                                                                                                                                                                                     |
| `o`                | Open Roles for a reply row only.                                                                                                                                                                                                                       |
| `g`                | Show the generated server command, including chosen model IDs and saved ordered prompt flags.                                                                                                                                                          |
| `r`                | Rescan local artifact state.                                                                                                                                                                                                                           |
| `x`                 | Cancel the local download; partial files remain available for explicit retry/resume.                                                                                                                                                                   |

Download progress/logs appear **inline below the right-hand model information** and remain visible after completion, failure or cancellation. After downloading, **Enter** selects the model. Selections persist automatically; there is no separate save confirmation or command popup. The optional `g` server command view does not remotely manage the server.

#### Reply roles and startup precedence

The reply-only `o` picker browses `.txt` files in the directory of the manifest default role. Qwen resolves `models/manifest.toml`'s `../roles/incident-reporting.txt` to the checked-in [`roles/incident-reporting.txt`](roles/incident-reporting.txt); this file is selected by default. Use arrows/`j`/`k` to browse, **Space** to toggle in selection order, **a** to add an arbitrary trusted local `.txt` path, **Enter** to validate/save, and **Esc** to cancel. Empty/invalid selections do not replace saved settings. `p` on Models previews the saved/default combination and hash.

Each source must be a regular, non-empty UTF-8 text file. Sources retain their exact text, including whitespace, and are concatenated in selected order with exact `\n\n` separators. The combined prompt is limited to **32 files** and **`MAX_PROMPT_CHARS` = 16,384 Unicode scalar values**, including separators. Its SHA-256 covers the exact combined UTF-8 bytes. The existing default **12,000-character aggregate system role + selected history + approved question** budget still applies. The TUI refuses to save a combination consuming that whole budget, and each runtime checks it against its configured input budget.

TUI config stores `reply_role_files` as model-ID → ordered absolute path lists. Saving Roles changes the next TUI/server start; active prompts remain loaded. **Enter** on Models selects the reply model and activates a changed selection automatically. The TUI loads the legacy `server_reply_model` key and the selected model’s saved role paths. Older configs without this map keep the manifest default. Dedicated `StartupChoices` keeps `stt_model`/`reply_model` and adds optional `reply_prompt_files` (empty by default). For example, a choices file placed directly in `pheme-va/` can contain:

```toml
stt_model = "whisper-large-v3-turbo"
reply_model = "qwen2.5-1.5b-instruct-q4-k-m"
reply_prompt_files = ["roles/incident-reporting.txt"]
```

Relative paths in `reply_prompt_files` resolve from the choices file's directory, not the server's working directory. That list requires a `reply_model` selection. Startup uses this precedence:

1. Explicit repeated **`--prompt-file`** sources, in flag order, or the legacy single-file **`--system-prompt`**. These options conflict with each other and override saved roles.
2. Saved ordered role files only when their saved reply model ID matches the selected model. Selecting a different explicit `--reply-model` never inherits the previous model's roles.
3. The selected reply entry's manifest `system_prompt` when no applicable role choice exists.

Raw **`--reply-path` requires explicit `--prompt-file` or `--system-prompt`** and never inherits saved/manifest roles. The server validates/loads the combined role once and reports ordered source paths and its hash at startup. Missing/invalid roles fail explicitly, never fall back to generic instructions. File edits and model/role choices require restart; restart loses in-memory web history. User requests cannot select paths, rewrite system instructions or authorize report tools.

### Stateless development HTTP service

```bash
cargo run --release -p server --features whisper -- \
  --model /path/to/models/transcript/whisper/ggml-large-v3-turbo.bin \
  --bind 127.0.0.1:8000 \
  --metrics-enabled \
  --incident-metrics \
  --resource-sampling
```

Then send a WAV:

```bash
curl -X POST http://127.0.0.1:8000/v1/transcribe \
  -H 'Content-Type: audio/wav' \
  --data-binary @recording.wav
```

Health endpoints are `GET /health` and `GET /ready`. `POST /v1/analyze` provides the conservative deterministic incident-field extractor for development; model-backed structured extraction remains behind the same replaceable boundary. `GET /v1/metrics/batches` drains locally collected metric batches. Set `PHEME_VA_METRICS_ENABLED=true` and `PHEME_VA_RESOURCE_SAMPLING=true` (or use the flags above) to collect metrics; `X-Run-ID` and `X-Experiment-ID` request headers control correlation.

## Real audio test

Any model-backed test is ignored by default because weights and speech fixtures must remain outside Git:

```bash
PHEME_VA_WHISPER_MODEL=/path/to/model \
PHEME_VA_TEST_AUDIO=/path/to/speech.wav \
PHEME_VA_TEST_LANGUAGE=en \
cargo test -p whispercpp --test audio -- --ignored --nocapture
```

The fixture should contain actual speech, not a generated tone. The ordinary `wav_audio_reaches_transcription_engine` test verifies the audio and orchestration path with a local fake recognizer.

## Mobile direction

The mobile target is an embedded library, not a separately launched server process:

```text
iOS AVAudioEngine / Android AudioRecord
              |
              v
        ffi
              |
              v
        core + selected STT adapter
```

The mobile host owns microphone permission, audio-session lifecycle, UI, and HTTP transport. Rust receives interleaved `f32` samples and returns text. When built with Whisper, the FFI also collects per-call metric batches; the host can toggle collection with `pheme_va_metrics_set_enabled` and drain JSON with `pheme_va_metrics_drain`, then forward those batches to the Go metrics endpoint. Native iOS/Android resource providers can implement the `metrics::ResourceSampler` trait in a later host integration. The FFI crate can be built as a static or dynamic library:

```bash
cargo build -p ffi --release --features whisper \
  --target aarch64-apple-ios
```

Apple linking, Metal/Core ML configuration, signing, and physical-device validation require macOS/Xcode. A successful Linux build is not evidence that iOS inference works. Android should use the same core through a native `.so`/C ABI layer after the iOS spike.

## Design notes

- Audio processing is in the core so all hosts use the same sample contract.
- A silence gate prevents a speech-recognizer call on empty input, but its thresholds are configurable and not yet device-validated.
- Dictionary prompts are bounded and deduplicated. Word overlap alone does not discard speech; only prompt-shaped continuation is treated as an echo.
- Decoder thresholds are heuristics, not factuality guarantees. Preserve raw text and measure false positives before changing them.
- Cleanup failure falls back to raw text. Incident extraction must not use rewritten text as the only evidence.
- The current LLM seam is a trait; no cloud provider is enabled by default and no model-specific cleanup prompt is hard-coded into the core.
- Runtime/model versions, latency, memory, power, and quality must be recorded during later device evaluation.
- Metrics are published internally through `metrics::MetricsHub`; TUI, tests, and host exporters subscribe without making the core depend on an API or UI.
- The Go metrics endpoint stores batches separately from benchmark results. Unsupported device values remain null with an explanation; CPU percentage is never treated as a power measurement.
