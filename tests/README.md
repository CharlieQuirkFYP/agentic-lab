# Cross-service voice checks

Normal unit tests stay beside their Go/Rust packages and under `web/tests/`.
This directory exercises the real Go → Pheme HTTP boundary with a tiny fake
stdio reply worker and the web's actual SSE parser/reducer. No weights, network
downloads, microphone, browser or TTS device are required.

Prerequisites: stable Rust, Go 1.25+, Node 22.18+, Bash. Build from each component
then run the bounded test from the repository root:

```bash
# pheme-va/
cargo build -p server

# api/
go build -o ../pheme-va/target/integration-api ./cmd/server

# repository root
node --test tests/voice-stack.test.mjs
```

The test starts both binaries on ephemeral loopback ports, installs a temporary
fake worker/model, bypasses personal saved startup choices, and tears down its
processes/temp files. It verifies:

- web input waits for review; TUI-style inspection and approval return corrected
  Unicode wording/deltas to that web's original response;
- TUI tests while web reviews do not appear in web streams/history/inspection;
- successful history reaches web follow-ups, while each test has fresh context;
- ordered local role files combine once at startup and reach every reply as the system prompt;
- one worker initialization serves repeated operations;
- test disconnect and web cancellation reach the worker; reset settles work;
- configured browser origins work through Go; untrusted origins cannot mutate
  the development conversation;
- no model-download HTTP endpoint exists.

The fake worker is a deterministic fixture, not a real model. This test does
**not** validate reply quality, resource/energy readings, native GGUF inference,
real speech recognition, terminal keyboard interaction, browser permissions or
audible playback. For native link checks build the combined server and worker:

```bash
# pheme-va/; needs a C/C++ compiler, CMake and libclang
cargo build -p server -p reply-native --features server/whisper
```

Real-model tests remain ignored/opt-in and take model/audio/prompt paths from
environment variables; see the [model setup](../pheme-va/models/README.md) and
[Pheme README](../pheme-va/README.md). Do not commit weights or recordings.
