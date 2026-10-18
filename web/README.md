# Agentic Lab Web

React + TypeScript + Vite development voice console with Tailwind, the existing shadcn button, and Lucide icons. `/voice` is the operational development view; `/benchmarks` preserves research-dashboard navigation and explicitly shows that real measurements/dashboard functionality are not available yet.

This increment is a question/reply development loop, **not** incident confirmation, report storage, or retrieval. Pheme owns the shared web conversation. Independent TUI tests are not a web event source.

## Development

Use Node 22.18+ (or a current supported Node release); tests use Node's built-in TypeScript stripping and test runner without another dependency.

```sh
npm ci
npm run dev
```

Start Go from `api/` with explicitly allowed local browser origins:

```sh
API_CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 go run ./cmd/server
```

This is required even with Vite's same-origin development proxy, because it forwards the browser's `Origin` header. Without an allowed origin, the Go development guard rejects browser mutations (TUI/non-browser requests still work). Vite proxies `/api/v1` to `http://127.0.0.1:8080`; the browser defaults to the same-origin `/api/v1` public Go base. It does not call Rust directly.

Optional `.env.local` configuration:

```dotenv
VITE_API_BASE_URL=http://localhost:8080/api/v1
```

Use the **API base**, not the `/voice` suffix. Vite environment variables are public browser configuration, never secrets. Browser requests carrying an Origin header need that exact origin allowed on the API, whether direct or proxied. Production hosting needs its own reverse proxy for `/api/v1`; Vite's development proxy is not part of the production build. Protect the shared development conversation before exposing it over a network: CORS is not authentication.

## Voice loop

1. **Start microphone** asks for permission and captures PCM in an AudioWorklet. **Stop & transcribe** finishes a mono PCM16 WAV and sends the complete clip; no MediaRecorder/WebM/Opus upload or streaming recognition is used. Pheme handles authoritative audio normalization/resampling. Text can also start a reviewable turn.
2. Edit the transcript and **Submit approved question**. Transcription never approves a question automatically. The server freezes the first accepted wording. If the TUI approves a correction, `question.approved` on the original web response replaces the local editor and disables conflicting edits/submission.
3. Reply deltas render as received, without simulating streaming from a final response. Completion replaces provisional text with the authoritative full reply. All text is rendered as text, not HTML.
4. **Stop generation / Cancel turn** sends a backend cancellation command. **Stop voice** only stops client playback. **Reset web context** requires confirmation and invalidates old callbacks before clearing history, so stale replies cannot restore cleared content. Navigation to Benchmarks discards microphone capture/stops current playback, but the turn controller remains mounted.

Client limits: 60 seconds of recording, 16 MiB encoded WAV, 4,000 UTF-16 code units for input/review, 64,000 for displayed replies, and 20 completed pairs in browser-view memory. Backend limits and model availability remain authoritative. The microphone is released on finish, discard, errors, page exit, and unmount, including permission requests that resolve after cancellation. No audio, transcript, reply, turn ID, or voice choice is persisted in browser storage.

Microphone capture needs localhost or HTTPS and AudioWorklet support. Plain HTTP on a LAN address usually cannot use the microphone. Text input still works without microphone permission.

## Playback and privacy

Client speech synthesis uses **only voices with `localService === true`**. There is no remote fallback. Install an OS/local browser voice if none is available. Local execution is a browser/OS assertion, not an independently verified offline guarantee; verify your target browser/device with networking disabled. Speech failures preserve answer text and never regenerate it. Playback stops before recording.

Auto-speech is eligible only for a completed reply delivered live through this web turn's original response while the voice view is open. Independent TUI tests, incremental/failed/cancelled replies, recovered results, and status reads do not trigger speech. **Replay** deliberately speaks a completed result without inference. Browser policy or voice availability may still block automatic playback; errors are shown honestly.

Audio/text are sent to the configured Go API. Using a mobile browser with a remote backend is not on-device inference.

## API contract and recovery

The client consumes the **POST response itself** using `fetch` and an incremental `TextDecoder`/SSE parser. It supports UTF-8 character boundaries, split CRLF delimiters, multiline data, comments/heartbeats, and terminal-frame cancellation. There is no EventSource/global subscription or inspection polling.

Relative to the public API base:

- `POST /voice/turns`: raw `audio/wav` or JSON `{ "text": "..." }`; a fresh `Idempotency-Key` is attached. SSE events are `turn.created`, `transcript.ready`, `question.approved`, `reply.started`, `reply.delta`, `reply.completed`, `turn.failed`, `reply.failed`, and `turn.cancelled`, correlated by `turn_id`.
- `POST /voice/turns/:id/submit`: JSON `{ "text": "..." }`, preserving exact edited wording. Generated text stays on the original response, not this command.
- `POST /voice/turns/:id/cancel` and `POST /voice/reset`: explicit mutation commands.
- `GET /voice/turns/:id`: turn status/result recovery only. Status is `transcribing`, `awaiting_review`, `generating`, `completed`, `failed`, or `cancelled`; text fields are `transcript`, nullable `approved_text`, and `reply`, with optional error/timings.
- `GET /voice/inspect`: initial model-readiness badges (`stt` and `reply`, each `{ name, ready }`) only. Snapshot history/current turn/role are deliberately not a web conversation feed. No web model downloads, selection, or prompt controls exist.

A dropped stream triggers one status read for its known turn ID. **Read turn status** performs subsequent deliberate GETs; it does not resume streaming, resend input, or auto-speak. If transport fails before the ID arrives, the page blocks starting another turn until an explicit reset, because receipt by the server is uncertain. Missing/expired turns and failed resets are shown explicitly, not silently rerun. Refreshing the browser loses its in-memory turn identity/history; it does not automatically recreate a question or restore/play inspection content.

## Verification

```sh
npm test
npm run lint
npm run build
```

Tests use Node's built-in runner, faked fetch responses, small PCM arrays, and an isolated AudioWorklet harness. They cover UTF-8/frame splitting, terminal streams, WAV headers/PCM/limits, downmix/flush, explicit approval and TUI wording, turn-ID isolation, failure/cancellation, public API requests/status recovery, and local-only voice selection. They do not download models or depend on the Go/Rust services.

Manual browser acceptance still requires the running Go/Pheme implementation and real browser microphone/TTS APIs: record and review; approve a TUI correction; verify live incremental text; cancel/reset; interrupt transport and recover without resubmission/autoplay; deny microphone access; verify local playback/offline behavior and no remote fallback. Power/energy/resource measurements and the full benchmark dashboard remain separate work.
