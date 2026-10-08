# Agentic Lab

## Goal

Agentic Lab is a university final year project in collaboration with KLASS. The project is building a local, speech-first incident-reporting system with clarification, human confirmation, and retrieval of previous reports.

The canonical voice-agent implementation is [`pheme-va/`](pheme-va/), a portable Rust workspace. It provides the audio pipeline, Whisper/whisper.cpp integration, host adapters, and the foundation for incident analysis and workflow execution. The project evaluates quality, latency, memory, CPU/GPU usage, power, energy, and thermal/endurance trade-offs on resource-constrained hardware.

The Go API remains the application-facing backend and benchmark coordinator. The web app provides a development voice console; the full research dashboard remains planned. The former Python implementation has been removed; the historical Python contract documentation is retained separately for reference only. Existing KLASS solutions are not being integrated. Vision/licence-plate monitoring is out of scope and interview development is deferred.

## Project Documentation

- [Requirements and scope](docs/requirements.md)
- [Planned system architecture](docs/architecture.md)
- [Benchmark methodology](docs/benchmark-methodology.md)
- [Implementation roadmap](docs/roadmap.md)
- [Current experiment API](docs/api/experiments.md)
- [Pheme VA README](pheme-va/README.md)
- [Contributor/agent instructions](AGENTS.md)

## Local Development

The active components are the Go API, the Pheme VA Rust workspace, and the web voice console. Both listeners default to loopback; protect them with HTTPS and access controls before network exposure.

### Go API

#### Prerequisites

- Go 1.25.0 installed
- Git
- The repository cloned locally

#### Setup

From the repository root, navigate to the API service:

```bash
cd api
go mod tidy
API_CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 go run ./cmd/server
```

The server runs at `http://localhost:8080`.

#### Verify the API

```bash
curl http://localhost:8080/health

curl -X POST \
  http://localhost:8080/api/v1/incidents/analyze \
  -H "Content-Type: application/json" \
  -d '{
    "transcript": "A vehicle collided with a barrier near the west entrance."
  }'
```

The public incident endpoint still uses Go's `MockIncidentAnalyzer`; do not interpret it as model-backed structured analysis. Dedicated voice routes now delegate to Pheme through `PHEME_VA_URL` (default `http://127.0.0.1:8000`). `API_ADDR` defaults to `127.0.0.1:8080`; `API_CORS_ORIGINS` must explicitly allow the browser origin, including when Vite/reverse-proxy forwards its `Origin` header. The command above allows the two local Vite origins; without it, TUI/non-browser calls still work but browser mutations are denied.

Run the Go checks from `api/`:

```bash
gofmt -l .
go vet ./...
go test ./...
```

### Pheme VA

Pheme VA requires the stable Rust toolchain. From the repository root:

```bash
cd pheme-va
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The workspace contains:

- `crates/core`: portable audio normalization, mono 16 kHz conversion, speech gating, dictionary prompts, transcript guards, replaceable adapter traits, and conservative incident extraction
- `crates/models/transcription/whispercpp`: in-process whisper.cpp model adapter
- `crates/models/transcription/zipformer`: optional LiteRT Zipformer CTC model adapter
- `crates/models/reasoning`: persistent stdio reply client plus the `reply-native` llama.cpp worker
- `crates/cli`: standalone STT bench and server-connected Web/Tests/Models/Telemetry TUI
- `crates/server`: development HTTP host around the same core
- `crates/ffi`: C ABI for native iOS/Android hosts

The core does not own a frontend hotkey, clipboard, microphone permission, or mobile UI. Hosts provide audio and control their own lifecycle.

#### Download the Whisper model

Model weights are ignored by Git and must not be committed. The initial CPU baseline is Whisper `large-v3-turbo`, stored by new downloads at `pheme-va/models/transcript/whisper/ggml-large-v3-turbo.bin`:

```bash
cd pheme-va
./scripts/download-model.sh
```

The script downloads the model from the whisper.cpp model repository and verifies SHA-256 before installing it. Run `./scripts/download-model.sh --list` for the selectable STT artifacts, pinned Qwen reply GGUF, and experimental SeaLLMs download. Existing legacy weights are not moved automatically; see the model README for migration before downloading again. Downloading an artifact does not activate a runtime or Cargo feature. See [`pheme-va/models/README.md`](pheme-va/models/README.md) for sources, checksums, status, and licensing reminders.

This model is used by the separate `whispercpp` crate, which is built on whisper.cpp. It is an initial baseline, not a validated mobile configuration. Smaller models, Zipformer variants, accelerator support, and device-specific settings still need to be evaluated.

#### Transcribe a WAV file

```bash
cargo run --release -p cli --features whisper -- \
  --stt-model whisper-large-v3-turbo \
  transcribe recording.wav \
  --language en \
  --dictionary KLASS,whisper.cpp,"west entrance"
```

The input may use a supported sample rate, channel count, or WAV sample format; Pheme normalizes it before the selected speech model receives it.

#### Run the development HTTP host

```bash
cargo run --release -p server --features whisper -- \
  --model models/transcript/whisper/ggml-large-v3-turbo.bin \
  --bind 127.0.0.1:8000
```

The host exposes:

```text
GET  /health
GET  /ready
POST /v1/transcribe   Content-Type: audio/wav
POST /v1/analyze      Content-Type: application/json
```

`/v1/transcribe` returns raw and processed transcript text, speech-gate status, segments, backend information, and processing time. `/v1/analyze` currently uses the conservative deterministic incident extractor; model-backed structured extraction is a separate adapter task.

#### Run the model-backed test

The real-model test is ignored by default because it requires local model weights and a speech recording:

```bash
PHEME_VA_WHISPER_MODEL="$PWD/models/transcript/whisper/ggml-large-v3-turbo.bin" \
PHEME_VA_TEST_AUDIO=/absolute/path/to/speech.wav \
PHEME_VA_TEST_LANGUAGE=en \
cargo test -p whispercpp --test audio -- --ignored --nocapture
```

A successful run confirms that the selected Whisper model loads through the separate whisper.cpp-backed model crate and produces non-empty transcript text. It does not by itself validate incident-report quality or target-device performance.

#### Shared voice conversation and TUI

On Linux, install the system audio development package required by `cpal` if necessary, for example `libasound2-dev` on Debian/Ubuntu:

From `pheme-va/`, build server and reply worker (requires CMake, a C/C++ compiler and libclang) and start the pair:

```bash
cargo build --release -p server -p reply-native --features server/whisper
./target/release/server \
  --stt-model whisper-large-v3-turbo \
  --reply-model qwen2.5-1.5b-instruct-q4-k-m
```

Explicit IDs directly download missing pinned artifacts at startup: approximately 1.62 GB for Whisper and 1.12 GB for Qwen. You can instead use the TUI's local Models downloader first. Weights load once; the server owns a stdio reply child to isolate incompatible Whisper/llama.cpp native libraries—not an extra HTTP service or reply feature flag.

With Go running on port 8080, attach the TUI:

```bash
cargo run --release -p cli -- tui --server-url http://127.0.0.1:8080
```

Use **w Web** to inspect/edit pending web transcripts, **b Tests** for isolated tests, **m Models** for the unified local catalog/downloads and next-start model/role choices, and **t Telemetry** for existing panels. Only Telemetry uses **1–4** for sub-tabs; **Esc** returns to the previous workspace. Web or TUI explicitly approves text; only the separate reply model answers. The web receives the answer through its own request and owns speech playback. TUI inspection is silent unless you explicitly replay.

Omit `--server-url` to retain standalone STT tests. See [Pheme setup and limits](pheme-va/README.md#shared-webtui-voice-loop) and [voice API](docs/api/voice.md). Model/prompt changes require restart; in-memory web context does not survive it. This loop does not save/finalize incident reports.

### Web

#### Prerequisites

- Node.js 22.18+ (tests use built-in TypeScript stripping)
- npm

#### Setup and verification

```bash
cd web
npm install
npm run dev
npm test
npm run lint
npm run build
```

The Vite development server runs at `http://localhost:5173`. Open `/voice` for microphone → review → streamed reply → client-side speech. Its `/api/v1` proxy calls Go, not Rust directly. Browser speech uses local-service voices only; availability/offline behavior depends on the OS/browser. The benchmark view marks real measurements/dashboard features as not implemented. See [web setup](web/README.md).

## Model and deployment notes

Whisper/whisper.cpp is the current speech-recognition baseline. Pheme keeps transcription and language-model inference behind replaceable boundaries so alternative local models can be evaluated. A concrete llama.cpp-compatible language-model adapter for structured incident extraction is still planned; the current rule-based extractor is deliberately conservative and does not invent facts.

The eventual target may be an iOS phone or NVIDIA Jetson, but Linux builds and local HTTP calls are not evidence of successful on-device deployment. Record model identity, checksum, runtime revision, quantization, device, timings, and measurement method for every evaluation.
