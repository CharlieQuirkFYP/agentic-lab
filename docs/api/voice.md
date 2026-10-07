# Development voice conversation API

Implemented development flow: Web → Go → Pheme. One in-memory web conversation, with no session setup, join flow, global subscriptions, database or model-management HTTP routes. Independent TUI tests share loaded compute but never use/update web history, inspection content or web response streams. This does **not** implement confirmed incident report storage/retrieval or replace the existing structured incident analyzer.

## Hosts and readiness

- Go public base: `http://127.0.0.1:8080/api/v1/voice`; configure `PHEME_VA_URL` for the private Rust host (default port 8000).
- Pheme: `http://127.0.0.1:8000/v1/voice`.
- Web uses its own request-scoped POST response. TUI polls inspection every 500 ms in a worker and submits only explicitly approved edits.
- `/health` is liveness. Existing Pheme `/ready` remains **STT readiness**, not combined reply readiness. Inspect `stt`/`reply` badges for the complete voice runtime.
- Both hosts bind loopback by default. Inspection contains potentially sensitive speech and has edit privileges; LAN exposure requires external authentication/access controls and HTTPS. Set exact `API_CORS_ORIGINS` values even behind Vite/reverse-proxy when it forwards the browser's Origin header; the default empty list denies browser-origin requests. For local Vite use `API_CORS_ORIGINS=http://localhost:5173,http://127.0.0.1:5173`. This origin guard is not authentication.

## Routes

The table paths are relative to the voice bases above. Go forwards status codes, JSON and SSE without rebuilding prompts or maintaining another conversation state machine.

| Method | Path                     | Behavior                                                                                            |
| ------ | ------------------------ | --------------------------------------------------------------------------------------------------- |
| POST   | `/turns`                 | WAV or text starts a web-owned turn; its direct SSE response spans transcription, review and answer |
| POST   | `/turns/:turn_id/submit` | Approve exact edited text for the pending web turn; JSON acknowledgement, not an answer stream      |
| GET    | `/turns/:turn_id`        | Read retained status/result, without replay or regeneration                                         |
| POST   | `/turns/:turn_id/cancel` | Signal cancellation; acknowledge current turn status                                                |
| POST   | `/reset`                 | Cancel/settle web work, clear web history/results; does not cancel isolated tests                   |
| GET    | `/inspect`               | Web history/current turn plus runtime badges; excludes test content                                 |
| POST   | `/test/reply`            | Stream a stateless isolated test using the bound role, never web history                            |
| POST   | `/transcribe`            | **Go only:** delegate stateless WAV test to Pheme `/v1/transcribe`                                  |

`turn_id` is request/result correlation, not a session or credential. No route downloads/selects/reloads a model or prompt. Only local TUI/script operations and explicit startup choices do that.

## Starting and approving a web turn

`POST /turns` accepts either:

- `Content-Type: audio/wav` (or `audio/x-wav`), raw complete WAV body, not multipart/WebM/Opus; or
- `Content-Type: application/json`, `{ "text": "There is smoke at the east entrance." }`.

Text input also enters review; it is not automatically answered. Voice-turn creation requires a ready reply model/role, and WAV additionally requires STT. Existing stateless transcription remains usable without a reply runtime. Only one pending/active web turn is accepted.

Optional `Idempotency-Key` is 1–128 printable non-space ASCII bytes. Identical known input acknowledges retained state without inference; conflicting reuse is `409`. The retry acknowledgement is a finite SSE snapshot, not a second live subscription. Read the identified turn with GET for recovery. Never automatically speak a recovered/snapshotted answer.

To approve from either web or TUI:

```http
POST /api/v1/voice/turns/turn_<id>/submit
Content-Type: application/json

{"text":"There is smoke at the west entrance."}
```

Pheme atomically checks turn identity, review state, input bounds and inference availability. It preserves the exact accepted text, including Unicode/whitespace; neither raw ASR text nor another client's draft substitutes for it. The first accepted submission freezes the question. An identical repeat returns known status; different wording conflicts. Busy inference leaves an unapproved turn reviewable for explicit retry. Re-recording/correcting an already approved question requires a new turn.

The answer stays on the **original web POST response**, even if TUI submitted the correction. The submit response promptly returns the turn object.

## Request-scoped SSE

A successful start uses `Content-Type: text/event-stream`. Go flushes upstream bytes, and Pheme emits heartbeat comments every 10 seconds during idle/review waits. Use streaming `fetch` for POST, not `EventSource` or a shared broadcast subscription. UTF-8 characters, delimiters and JSON can cross network chunks.

```text
event: turn.created
data: {"turn_id":"turn_example"}

event: transcript.ready
data: {"turn_id":"turn_example","text":"There is smoke at the east entrance."}

event: question.approved
data: {"turn_id":"turn_example","text":"There is smoke at the west entrance."}

event: reply.started
data: {"turn_id":"turn_example"}

event: reply.delta
data: {"turn_id":"turn_example","text":"Is anyone "}

event: reply.delta
data: {"turn_id":"turn_example","text":"in immediate danger?"}

event: reply.completed
data: {"turn_id":"turn_example","text":"Is anyone in immediate danger?"}
```

Deltas come from actual native generation and are provisional. Completion supplies authoritative full text. `turn.failed` includes `{ "turn_id": "...", "error": { "code": "...", "message": "..." } }`; `turn.cancelled` terminates a cancelled turn. Failed/empty/cancelled answers commit no successful user/assistant history pair. Preserve partial text as diagnostic display, not a completed answer.

Isolated `/test/reply` accepts JSON `{ "text": "..." }` and emits the same reply events **without a web turn ID**. Its role is identical but its context starts fresh. Closing this test transport sends native cancellation to only that test. Closing TUI inspection affects no web work.

## Inspection and recovery

`GET /inspect` returns:

```json
{
  "history": [
    { "role": "user", "content": "An earlier approved question" },
    { "role": "assistant", "content": "Its completed reply" }
  ],
  "current_turn": {
    "turn_id": "turn_example",
    "status": "generating",
    "transcript": "The original processed transcript",
    "approved_text": "The corrected approved question",
    "reply": "Accumulated partial answer",
    "error": null,
    "timings": { "review_ms": 1200.5, "first_text_ms": 220.0 },
    "role_sha256": "<exact-byte combined prompt hash>"
  },
  "stt": { "name": "<active runtime name>", "ready": true },
  "reply": { "name": "<active reply model name>", "ready": true },
  "role": {
    "name": "<first bound role source path>",
    "sha256": "<exact-byte combined prompt hash>"
  },
  "busy": true
}
```

`current_turn` is nullable. States: `transcribing`, `awaiting_review`, `generating`, `completed`, `failed`, `cancelled`. `GET /turns/:id` returns just the turn object. Timings become available as stages settle; absent values are unavailable, not zero. TUI replaces snapshots rather than appending the same partial answer; its local dirty editor stays bound to its original turn ID.

An accepted web inference may finish after browser transport loss; bounded snapshots/results remain recoverable. A slow browser does not block inference or create unbounded buffering: the stream closes and clients recover by GET. The web performs one recovery read, then offers explicit status reads/replay; it does not automatically reopen a global stream, regenerate a question or speak a recovered answer. A missing/expired/reset turn returns an explicit error. Restart loses in-memory context, and clients do not resend it silently.

## Limits, cancellation and errors

Default Pheme limits (selected values configurable at startup):

- 16 MiB body; JSON text bodies additionally limited to 64 KiB. Go's outer upload ceiling is 32 MiB; the stricter Pheme limit still applies.
- 120-second audio cap; the current web client records at most 60 seconds.
- 12,000 Unicode scalar values for aggregate system instruction + selected history + approved question; 4,000 reply characters.
- 4,096 model context tokens and 512 maximum output tokens; actual token overflow is rejected by the adapter.
- Six successful history pairs, 32 retained turn results.
- 300-second review, 120-second generation, 180-second transcription deadlines.
- One active inference across STT/web replies/tests. Human review releases compute.
- 256 lifetime idempotency keys: expired results retain bounded tombstones, including after reset. Ledger saturation returns `429` rather than rerunning an old request. Explicit server restart clears this development ledger and context.

Reply cancellation reaches the native worker's atomic/llama.cpp abort callback. Normal cancellation drains the terminal worker event and keeps weights loaded. An unresponsive/crashed/protocol-invalid worker is killed/reaped and answering becomes not-ready until restart. Current STT has no native abort interface: its result is discarded after cancellation/timeout, but compute stays reserved until the call settles. Reset waits for that settlement rather than allowing stale work into new context.

Custom failures use `{ "error": { "code": "...", "message": "..." } }`. Common statuses: invalid text/JSON `400`, stale/frozen/busy turn `409`, expired known retry `410`, body limit `413`, unsupported media `415`, retry ledger capacity `429`, missing runtime/role `503`; stream-time failures use terminal events. Unknown retained turns are `404`. Axum body/extractor rejections can retain plain-text framework responses. Go sanitizes connection errors as `upstream_unavailable`/`upstream_timeout` without exposing internal URLs/dial details.

## Models, roles, playback and metrics

Local registry `pheme-va/models/manifest.toml` retains `[[models]]` entries. `purpose`, `download_id` and manifest-relative `system_prompt` distinguish ASR artifacts from instruction replies. The starter Qwen default is `../roles/incident-reporting.txt`, resolving to `pheme-va/roles/incident-reporting.txt`; approved text never goes back to Whisper for an answer.

The single local TUI Models page (`m`) covers both manifest purposes. Enter confirms a network download for missing artifacts; available STT loads/prepares only in standalone mode, never in connected clients. Available reply entries instead provide startup guidance. `s` hashes/confirms next-start choices, `o` opens a reply-only `.txt` Roles picker, `p` previews the combined role/hash, and `g` shows the generated command. Downloads remain inline below the right-hand model information, including their terminal status. These controls do not change the HTTP contract or an active server.

Trusted roles are startup-only: combine 1–32 validated non-empty UTF-8 files in selected order with exact `\n\n` separators, preserving source whitespace. `MAX_PROMPT_CHARS` is 16,384 Unicode scalar values including separators; the existing 12,000-character aggregate role/history/question budget still applies. `role.name` identifies the first source, while `role.sha256` and turn `role_sha256` hash the exact combined UTF-8 bytes. Ordered source paths are reported at startup, not a new inspection field.

TUI config stores model-ID → absolute path lists in `reply_role_files`; dedicated `StartupChoices` adds optional `reply_prompt_files`, whose relative paths resolve from the choices file directory. Repeated `--prompt-file` flags override saved matching-model roles and conflict with single-file `--system-prompt`. A different explicit reply model does not inherit the previous model's roles; without a saved/explicit role choice it uses its manifest default. Raw `--reply-path` requires explicit prompt flags. See [local controls and startup setup](../../pheme-va/README.md#unified-models-page). User requests cannot supply paths or replace system instructions. Model text is assistance, not authorization to save/dispatch/execute actions.

The server owns a persistent stdio `pheme-reply-worker` to isolate incompatible native GGML libraries. It must be built/shipped beside the server or located through trusted `PHEME_VA_REPLY_WORKER`. No client or HTTP endpoint builds it. See [setup](../../pheme-va/README.md#shared-webtui-voice-loop).

The server returns text only. Web speaks a validated live completion through browser-reported local voices; TUI inspection is silent and tests/replay optionally use local `espeak`. TTS failure preserves text and never regenerates a response. Restored results never automatically speak. Local-service browser voice flags are not an independently verified offline guarantee.

Existing metrics batches separate model loads, human review, first-text/full generation, total turn duration, operation origin, status, character counts, bounded history/truncation and role hash. Logs/metrics contain no question/reply text by default. Token counts and child-process CPU/RAM remain explicitly unavailable; server-process sampling is not a measurement of its native reply child. Client playback outcomes stay local, not a server-side power/energy measurement.
