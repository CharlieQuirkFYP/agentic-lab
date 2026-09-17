# Pheme VA

Portable, backend-first audio-to-text core for Agentic Lab. The core accepts audio and returns transcript text; it does not own a hotkey, clipboard, UI, or mobile navigation.

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

The repository contains seven crates:

| Crate        | Purpose                                                        |
| ------------ | -------------------------------------------------------------- |
| `core`       | Platform-independent pipeline and adapter traits               |
| `metrics`    | Per-run pub/sub metrics, resource samplers, and batch contract |
| `cli`        | WAV client and small terminal microphone recorder              |
| `server`     | Development HTTP wrapper around the same core                  |
| `ffi`        | Small C ABI for iOS/Android hosts (`include/pheme_va.h`)       |
| `whispercpp` | `whisper.cpp` model adapter                                    |
| `zipformer`  | Optional LiteRT Zipformer CTC model adapter                    |

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
- A ratatui-based local TUI with first-run onboarding, manifest model selection, worker-owned model switching, WAV browsing, live microphone capture, final transcript display, metrics graphs, and structured logs.
- A development HTTP wrapper accepting `audio/wav` and returning the serialized transcription result.
- A portable `metrics` crate with synchronous pub/sub, per-run filtering, application timings/status events, resource snapshots, explicit unavailable values, and batched export data.
- Linux/macOS/Windows process/system CPU and RAM sampling through `sysinfo`, with extension points for native mobile sensors and device power/thermal providers.
- Core and incident-analysis instrumentation that keeps pure text analysis separate from transcription.
- A C ABI bridge suitable for a thin Swift/Kotlin host, including JSON metric-batch draining.

The default deterministic tests do not download models or require an inference runtime.

## Verify the core

From `pheme-va/`:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Use Whisper locally

The local `models/` directory contains `whisper/ggml-large-v3-turbo.bin`, downloaded from the whisper.cpp Hugging Face repository. Run `./scripts/download-model.sh` to reproduce the setup; its checksum and source are recorded in [`models/README.md`](models/README.md). `large-v3-turbo` is an initial CPU baseline, not a validated mobile configuration; compare it against the LiteRT Zipformer adapter on the target device.

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

### Local TUI test bench

On Linux, install the system audio development package required by `cpal` if it is missing, for example `libasound2-dev` on Debian/Ubuntu. Start the TUI from `pheme-va`:

```bash
cargo run --release -p cli -- tui
```

The first launch opens onboarding and lets you choose a manifest model ID. The WAV browser defaults to `samples/` when launched from `pheme-va` (`pheme-va/samples/` when launched from the repository root). Use `--audio-directory` to override it or `tui --reconfigure` to reopen setup.

Select a model with Enter. The picker reports whether its adapter is compiled into the launcher, already cached, being checked, or needs preparation. A cached status means the validated backend can be reused without invoking Cargo build; selecting it performs the quick launcher restart needed to enter that feature-enabled executable. Cache misses build in the background, preserving already-enabled adapters, then restart automatically. Existing pre-cache builds require one new build to populate the cache. On Unix, the preparation screen streams actual Cargo/native output and automatically follows the newest lines. Press `t` for full logs or Escape on the preparation screen to cancel; failure leaves the current model available. Settings and saved run reports survive normal exits and automatic backend restarts. Live metrics, logs, and retry audio remain in memory only.

Automatic setup requires the original source checkout, Cargo/Rust, and native build dependencies. Known Whisper/Zipformer model IDs can download missing pinned, checksum-verified artifacts through `scripts/download-model.sh`. Cargo may also download dependencies or the LiteRT runtime. Backend cache generations live under `target/tui-adapters/cache-v2/`; Unix builds reuse a locked `target/tui-adapters/build/` for incremental compilation. Neither overwrites the running executable. This is a development convenience, not runtime compilation for mobile deployments.

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

The test bench supports `[f]` WAV selection, `[l]` microphone recording, `[n]` next WAV, `[r]` retry, `[m]` model switching, and `[t]` telemetry. `[1–4]` selects Overview, Metrics, Runs, or Logs. Runs opens a list first; Enter opens a report with individually spaced metadata, transcript/raw transcript, and a scrollable run-only metric snapshot. Arrows or `j/k` select a snapshot metric; Enter opens its history/details and Escape returns to the report. PageUp/PageDown scroll metadata/details; `[` / `]` switches reports. Graphs are not shown until requested. `[f]`, `/`, or Ctrl+F edits shared search; `[x]` clears it. Missing readings remain unavailable, not zero. On Unix, native stderr (including Whisper/ALSA diagnostics) is captured into bounded logs instead of corrupting the screen; capture is not yet implemented on other platforms.

#### Saved run history and privacy

Completed and failed Runs reports persist locally, including processed/raw transcripts, source filenames, model/runtime/revision metadata, timestamps, errors, and raw metric events (including unavailable reasons). **This can contain sensitive speech and paths.** No recordings or audio buffers are written to history. The backend-specific `TranscriptionResult` is not restored; historical report display uses the saved metadata and events.

History is a versioned JSON snapshot at `$XDG_STATE_HOME/pheme-va/tui-history.json` (absolute XDG path), falling back to `$HOME/.local/state/pheme-va/tui-history.json`. This is bounded development-console history, not the planned SQLite incident/session database; it reuses the existing JSON dependency rather than adding a database runtime. Keep one TUI session per history path. On Unix, newly created state directories use mode `0700` and snapshot files `0600`; this is not encryption. Existing parent directory permissions are not changed.

The newest 100 reports and newest 10,000 events per report are retained. Snapshots have a 64 MiB size ceiling; an oversized save reports an error and retains the previous snapshot. Writes run on a background thread, coalesce pending snapshots, and use file sync plus atomic replacement. Completed/failed reports are queued on the next TUI tick; shutdown drains worker results and flushes history before any automatic restart. Forced termination can still lose an unflushed update. Malformed, oversized, or unsupported history prevents startup without overwriting the file: move it aside for inspection or explicitly remove it to start fresh. Write/clear failures appear in status and logs, and a failed shutdown flush prevents automatic restart.

In Runs, press `c`, then `y` to delete **all** saved reports (not only filtered reports); Escape cancels. Clearing is disabled while a request or recording is active. Reports disappear only after deletion succeeds; other input is paused while deletion is in flight. This does not delete source WAVs, memory-only retry audio, or external backups, and is not secure erasure.

Resource readings include process/system CPU and RAM, available component temperatures, and Linux DRM GPU utilization and single-battery capacity drain. Process CPU can exceed 100% because it sums core utilization. GPU is the busiest readable card, not per-process use; temperature is the hottest reported component, not ambient temperature. Battery drain is a signed percentage-point decrease since the sampler's first valid reading (negative while charging), with coarse sensor resolution. Whole-device power and energy remain unavailable without a verified provider; component or battery-terminal power is not silently relabeled.

The current desktop sampler deliberately does not implement a whole-device power provider: it returns unavailable even if Linux exposes RAPL or battery `power_now`. RAPL measures CPU package/core domains, while battery power measures battery-terminal charge/discharge (particularly misleading as total consumption while plugged in). Joules and watt-hours are integrated from verified whole-device watts, so they are unavailable for the same reason. Enabling these metrics requires a calibrated power provider with a documented measurement boundary, not elevated permissions or a model change.

Current manifest models return a final transcript after a complete clip; they do not provide partial live words. This is intentionally a local development and model-evaluation console, not the production UI or a global desktop hotkey implementation.

### Development HTTP service

```bash
cargo run --release -p server --features whisper -- \
  --model /path/to/models/whisper/ggml-large-v3-turbo.bin \
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
