# Agentic Lab

## Goal

Agentic Lab is a university final year project in collaboration with KLASS. The project is building a local, speech-first incident-reporting system with clarification, human confirmation, and retrieval of previous reports.

The canonical voice-agent implementation is [`pheme-va/`](pheme-va/), a portable Rust workspace. It provides the audio pipeline, Whisper/whisper.cpp integration, host adapters, and the foundation for incident analysis and workflow execution. The project evaluates quality, latency, memory, CPU/GPU usage, power, energy, and thermal/endurance trade-offs on resource-constrained hardware.

The Go API remains the application-facing backend and benchmark coordinator. The web app will provide a development voice console and a research dashboard. The former Python implementation has been removed; the historical Python contract documentation is retained separately for reference only. Existing KLASS solutions are not being integrated. Vision/licence-plate monitoring is out of scope and interview development is deferred.

## Project Documentation

- [Requirements and scope](docs/requirements.md)
- [Planned system architecture](docs/architecture.md)
- [Benchmark methodology](docs/benchmark-methodology.md)
- [Implementation roadmap](docs/roadmap.md)
- [Current experiment API](docs/api/experiments.md)
- [Pheme VA README](pheme-va/README.md)
- [Contributor/agent instructions](AGENTS.md)

## Local Development

The active components are the Go API, the Pheme VA Rust workspace, and the web scaffold.

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
go run ./cmd/server
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

The public incident endpoint still uses the Go `MockIncidentAnalyzer`. The Pheme client integration is the next backend integration step; do not interpret the current placeholder response as model-backed analysis.

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
- `crates/models/whispercpp`: in-process whisper.cpp model adapter
- `crates/models/zipformer`: optional LiteRT Zipformer CTC model adapter
- `crates/cli`: WAV transcription and a small terminal microphone recorder
- `crates/server`: development HTTP host around the same core
- `crates/ffi`: C ABI for native iOS/Android hosts

The core does not own a frontend hotkey, clipboard, microphone permission, or mobile UI. Hosts provide audio and control their own lifecycle.

#### Download the Whisper model

Model weights are ignored by Git and must not be committed. The initial CPU baseline is Whisper `large-v3-turbo`, stored locally at `pheme-va/models/whisper/ggml-large-v3-turbo.bin`:

```bash
cd pheme-va
./scripts/download-model.sh
```

The script downloads the model from the whisper.cpp model repository and verifies SHA-256 before installing it. Run `./scripts/download-model.sh --list` for the explicitly selectable LiteRT Zipformer artifacts and experimental SeaLLMs download. Downloading an artifact does not activate a runtime or Cargo feature. See [`pheme-va/models/README.md`](pheme-va/models/README.md) for sources, checksums, status, and licensing reminders.

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
  --model models/whisper/ggml-large-v3-turbo.bin \
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
PHEME_VA_WHISPER_MODEL="$PWD/models/whisper/ggml-large-v3-turbo.bin" \
PHEME_VA_TEST_AUDIO=/absolute/path/to/speech.wav \
PHEME_VA_TEST_LANGUAGE=en \
cargo test -p whispercpp --test audio -- --ignored --nocapture
```

A successful run confirms that the selected Whisper model loads through the separate whisper.cpp-backed model crate and produces non-empty transcript text. It does not by itself validate incident-report quality or target-device performance.

#### Terminal microphone recorder

On Linux, install the system audio development package required by `cpal` if necessary, for example `libasound2-dev` on Debian/Ubuntu:

```bash
cargo run --release -p cli --features whisper -- \
  --stt-model whisper-large-v3-turbo tui
```

Press Enter or Space to start and stop recording, and `q` to quit. This is a development TUI, not the product UI.

### Web

#### Prerequisites

- Node.js 20.19+ or 22.12+
- npm

#### Setup and verification

```bash
cd web
npm install
npm run dev
npm run lint
npm run build
```

The Vite development server runs at `http://localhost:5173`. The voice console and benchmark dashboard are still scaffolding.

## Model and deployment notes

Whisper/whisper.cpp is the current speech-recognition baseline. Pheme keeps transcription and language-model inference behind replaceable boundaries so alternative local models can be evaluated. A concrete llama.cpp-compatible language-model adapter for structured incident extraction is still planned; the current rule-based extractor is deliberately conservative and does not invent facts.

The eventual target may be an iOS phone or NVIDIA Jetson, but Linux builds and local HTTP calls are not evidence of successful on-device deployment. Record model identity, checksum, runtime revision, quantization, device, timings, and measurement method for every evaluation.
