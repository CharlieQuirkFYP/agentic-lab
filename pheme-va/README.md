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

The first launch opens onboarding and lets you choose a manifest model ID. The WAV browser defaults to `local/audio/` when launched from `pheme-va` (`pheme-va/local/audio/` when launched from the repository root). Use `--audio-directory` to override it or `tui --reconfigure` to reopen setup.

Select a model with Enter. If its adapter is not compiled, the TUI builds it in the background, preserving already-enabled adapters, then automatically restarts with that model. Press `t` for compiler logs or Escape on the build screen to cancel; build failure leaves the current model available. Settings are retained, but restart clears in-memory results, metrics, and logs.

Automatic builds require the original source checkout, Cargo/Rust, and the native build dependencies. Cargo may download dependencies or the LiteRT runtime; **model weights are not downloaded automatically**. Builds use `target/tui-adapters/<host-triple>/release/cli` and do not overwrite the original executable. This is a development convenience, not runtime compilation for mobile deployments.

To avoid the initial build/restart, optionally compile both adapters up front:

```bash
cargo run --release -p cli --features 'whisper,zipformer' -- \
  --model-manifest models/manifest.toml \
  --stt-model whisper-large-v3-turbo \
  tui
```

If the Zipformer artifacts are not present, download one explicitly before selecting it:

```bash
./scripts/download-model.sh zipformer-small
```

The test bench supports `[f]` WAV selection, `[l]` microphone recording, `[n]` next WAV, `[r]` retry, `[m]` manifest model switching, and `[t]` metrics/logs. In telemetry, `[1–4]` changes tabs, `[` / `]` selects a retained run, and arrows or `j/k` select categorized metric series in Metrics/Graphs. Each series shows its latest value and timestamped progression, distinguished by scope, source, and unit. `[f]` or `/` edits one shared filter across Overview, Metrics, Graphs, and Logs; `[x]` clears it. Missing readings are not plotted as zero. On Unix, native stderr (including Whisper/ALSA diagnostics) is captured into bounded logs instead of corrupting the screen; capture is not yet implemented on other platforms.

Resource readings include process/system CPU and RAM, available component temperatures, and Linux DRM GPU utilization and single-battery capacity drain. Process CPU can exceed 100% because it sums core utilization. GPU is the busiest readable card, not per-process use; temperature is the hottest reported component, not ambient temperature. Battery drain is a signed percentage-point decrease since the sampler's first valid reading (negative while charging), with coarse sensor resolution. Whole-device power and energy remain unavailable without a verified provider; component or battery-terminal power is not silently relabeled.

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
