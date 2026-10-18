# Pheme voice replies — web-led loop and TUI inspection

**Status:** Development baseline implemented. Default Rust/Go/web and fake-worker cross-service checks use local fakes/fixtures rather than model weights. Real reply-model quality/latency, live microphone/TTS acceptance, and benchmark measurements remain unverified.

## Implementation notes

- `crates/core/src/conversation.rs` and `registry.rs` provide role-aware bounded requests, validation, catalog/prompt loading, and startup choices. `crates/server/src/voice/` owns the web turn/history; Go only delegates.
- `reply-model` is a normal dependency. Its server-owned `pheme-reply-worker` stdio child alone links llama.cpp because directly linking the current Whisper and llama.cpp GGML libraries produces incompatible duplicate symbols. This is process isolation within Pheme, not another HTTP service or feature-enable flag. Build/ship server and worker together; weights stay resident across turns. Include the child in resource accounting.
- The pinned starter reply entry is `qwen2.5-1.5b-instruct-q4-k-m`, approximately 1.12 GB, Apache-2.0; exact revision/SHA-256 are in the existing manifest. Its shared incident role file loads at startup. Selection is replaceable and is not a validated device configuration.
- Web uses the Go public API; connected TUI uses `tui --server-url` (Go defaults to port 8080). Main navigation is alphabetic: `w` Web, `b` Tests/back, `m` Models, `t` Telemetry. Only Telemetry retains 1–4 sub-tab controls, with Esc returning to the previous view. One manifest-backed Models page serves standalone and connected modes. Downloads remain direct local scripts/startup only; no model-management routes exist.
- Native reply cancellation is implemented; current STT has no native abort boundary, so cancellation/timeouts discard its result but retain the compute reservation until STT settles. Changed prompts/models apply on server restart, not live reload.
- Server metrics cover stage timings, status, counts and role hash. Native-child CPU/RAM and token counts are explicitly unavailable; server-process readings do not stand in for the child. Browser/TUI playback status is local, not a claimed server-side measurement.
- [Pheme setup](../../pheme-va/README.md#shared-webtui-voice-loop), [implemented voice API](../api/voice.md), and [cross-service verification](../../tests/README.md) describe the runnable contract. Wireframes below retain design intent; candidate names/numbers are illustrative.

## Intended behavior

Run one Pheme VA server. The **web owns its request/response conversation loop**. The TUI can inspect that web conversation, edit a pending web transcript, and explicitly send the corrected text to the reply model for that same web turn.

Independent TUI tests are separate: their transcripts and replies return only to the TUI and never update the web conversation. There is no global broadcast feed, session creation, session selection, or join flow.

```text
Web records a question and starts its request
        ↓
Server transcribes and returns the transcript to the web
        ↓
TUI can inspect the pending web turn
        ↓
Web OR TUI edits and explicitly approves that transcript
        ↓
Reply model answers the approved question
        ↓
Answer streams through the original web request
        ↓
Web displays/speaks it; TUI can inspect the answer

Independent TUI test → test endpoint → result only to TUI
```

**Who starts the turn and who approves its text are different things.** A TUI correction to a web-originated question helps complete the web's request; it does not turn it into a TUI-owned conversation or redirect the answer/playback to the TUI.

Whisper converts speech to text. The separate instruction/chat model answers approved text. Client-side TTS speaks the completed answer. Transcription never automatically invokes the reply model.

## Foundation and implemented additions

- `pheme-va/crates/core/src/engine.rs` already handles audio normalization, speech gating, transcription, and optional cleanup. Current CLI/server wiring uses `RuleBasedFormatter`, not an answering LLM.
- `LanguageModel`/`LlmTextCleaner` remain cleanup boundaries. Answering uses the separate `ConversationModel` trait and does not change transcription cleanup.
- The CLI has a persistent `ratatui` TUI with microphone capture, WAV browsing, background inference, model selection, telemetry, logs, and saved run reports. Extend it rather than replace it.
- The Rust HTTP host retains `/health`, `/ready`, `/v1/transcribe`, `/v1/analyze`, and `/v1/metrics/batches`, alongside the voice routes below. `/v1/analyze` remains deterministic incident extraction, not conversational answering.
- Go delegates the dedicated voice routes to Pheme; `MockIncidentAnalyzer` and mock benchmarks are unchanged. The React/TypeScript/Vite web console implements WAV capture, review, streamed replies and client speech; its benchmark page marks measurements as not implemented.
- Current audio uploads must be WAV. Browser WebM/Opus output is not accepted by the existing audio pipeline.

See [architecture](../architecture.md), [current API contract](../api/voice-agent.md), and [Pheme README](../../pheme-va/README.md). This development voice loop does not implement the separately planned incident confirmation, report storage, or retrieval workflow.

## Backend ownership and test isolation

```text
Web request/response loop       TUI inspection and edit controls
             \                    /
              Go API and Pheme client
                         ↓
                 One Pheme VA server
                 web conversation state
                         ↓
              Shared STT and reply runtimes
                         ↑
              Isolated TUI test requests
              results only to their caller
```

Pheme owns bounded successful web conversation history, the system instruction, and the current web turn's transcript, approved question, partial/final answer, and status. Go forwards commands/streams and does not build prompts or maintain another authoritative conversation state machine.

The TUI observes this state through a simple inspection endpoint. A web turn has a lightweight `turn_id` so an edit can target the right pending question. This is correlation for one request, not a session identifier or create/join workflow.

Keep web and test paths separate in the backend, not merely hidden by frontend rendering:

- Web turns use the web conversation history and become inspectable in the TUI.
- TUI transcription/reply tests use stateless or temporary test input/context. They do not read/write web history, replace the pending web transcript, or emit into a web response stream.
- Isolation is selected by the endpoint, not by trusting a `client_id` or filtering a global feed in the browser.
- Tests can reuse the same loaded model weights. Reset per-call inference/KV state appropriately so test prompts and outputs cannot leak into web turns.
- Shared compute may affect latency or produce a busy response. Result/history isolation is not a promise of separate hardware resources.

Load the selected STT/reply models in the server and reuse them until an explicit model change. Opening a client, polling, and ordinary turns never reload weights. Allow one active inference operation at a time initially; reject overlap with a clear busy result rather than add scheduling infrastructure. Keep one pending/active web turn. Waiting for human review must not hold an inference lock, so an isolated test can run while the web is reviewing.

Run inference off HTTP/UI threads and keep state locks brief. Bound recording size/duration, approved input, output length, model context, and retained history/results. Commit a web user/assistant pair only after a valid answer completes; failed/cancelled generation is not successful history.

Web owns normal conversation reset/cancel controls. Reset settles/cancels active work before clearing web history so delayed callbacks cannot repopulate it. A TUI test never resets the web context. Context is in memory and lost on server restart; clients must not silently resend old questions.

## Model files and runtime integration

Organize local model artifacts by purpose:

```text
pheme-va/models/
├── README.md
├── manifest.toml
├── transcript/
│   ├── whisper/
│   │   └── ggml-large-v3-turbo.bin
│   └── zipformer/
│       ├── bpe.model
│       ├── tokens.txt
│       └── <variant>/model.tflite
└── reply/
    └── <selected-reply-model>.gguf
```

- All supported transcription weights/tokenizer artifacts belong under `models/transcript/`; answering weights belong under `models/reply/`.
- These are artifact directories. Adapter source remains under `crates/models/` and portable interfaces stay in `crates/core/`.
- Keep model IDs stable when reorganizing transcription artifacts. Update `models/manifest.toml`, `scripts/download-model.sh`, setup documentation, and path-dependent tests/configuration together when implementing the move.
- Legacy local files may still live under `models/whisper/` and `models/zipformer/`. No automatic move/delete is performed. Reuse explicit paths or deliberately migrate files as documented in `models/README.md` rather than unnecessarily redownload them.
- Record source, revision, checksum, quantization, and license for the selected reply model. Keep weights/recordings out of Git and retain or extend ignore rules as necessary.

**The reply adapter is a normal server dependency, not an optional Cargo feature.** Normal server builds include reply support; users configure model paths/settings at runtime rather than rebuild with an enable-reply flag. Existing unrelated STT build options are not being removed by this plan.

Use the replaceable `ConversationModel` boundary without coupling core to a concrete runtime. The starter adapter uses pinned `llama-cpp-2`/sys `0.1.158` with Qwen2.5-1.5B-Instruct Q4_K_M and native cancellation. The persistent stdio worker keeps conflicting GGML symbols out of the HTTP host; no duplicate-symbol suppression or incompatible shared-GGML ABI is used.

Server configuration supplies transcription/reply paths, thread/context limits, maximum output tokens, generation settings, and the system-prompt file. Clients configure API URL and local TTS. Preserve standalone CLI `--stt-model`/`--model-manifest` and server `--model` compatibility.

Measure startup time, memory, first-text/full-reply latency, and quality on the actual host. A normal reply dependency may require native build dependencies in CI, but ordinary tests do not load/download weights. This desktop server is not proof of on-device mobile inference.

## Existing model registry and role bindings

Use the existing **`pheme-va/models/manifest.toml` and its `[[models]]` format** as the source of truth. Extend `ModelManifest`/`ModelEntry` and `ModelCatalog`, not a second registry, model API, or hard-coded TUI candidate list. Keep existing `id`, `family`, `model`, tokenizer/token paths, `repository`, `revision`, checksums, sizes, languages, runtime, and streaming metadata.

The registry extension uses optional fields in the same entries:

- `purpose`: `transcript` or `reply`. Legacy known Whisper/Zipformer entries remain transcription entries when this field is absent; do not infer that every new family is a transcriber.
- `download_id`: the allowlisted argument passed directly to `scripts/download-model.sh`. This handles the current Whisper registry ID `whisper-large-v3-turbo` versus script target `whisper`; Zipformer targets already match their IDs.
- `system_prompt`: the local role-instruction file for a reply entry. Resolve this new field relative to the manifest directory. Transcription entries have no conversational system prompt.

Schema excerpts for additional fields, not complete entries or fabricated pinned models:

```toml
# Existing Whisper entry:
purpose = "transcript"
download_id = "whisper"
```

```toml
# Pinned starter reply entry:
purpose = "reply"
download_id = "qwen2.5-1.5b-instruct-q4-k-m"
system_prompt = "../roles/incident-reporting.txt"
```

The TUI discovers model rows from `[[models]]`, groups by purpose, and derives local artifact state from their paths. Enable Download only for entries with a recognized script target and complete pinned source/revision/checksum metadata; show a clear Not downloadable reason otherwise. Verify multi-file STT artifacts as a bundle. Download availability, installed files, runtime support, prompt validity, startup selection, and active readiness are distinct states.

Remove the need for a separate hard-coded TUI download mapping once these fields are supported, retaining explicit legacy aliases during migration. The script still validates allowlisted targets and trusted pinned artifacts; tests check registry/script target consistency. Do not parse arbitrary TOML into shell commands or let a new registry entry execute arbitrary code. Registry-driven discovery does not imply every downloaded family has an implemented runtime adapter.

Each reply entry binds its default role through `system_prompt`. Multiple reply models can reference the same incident role file, making comparisons use consistent instructions rather than copied prompts. The reply-only local Roles picker can save an ordered combination of trusted `.txt` files per model for the next server start. Model-specific chat templates stay in adapters; no profile/session framework is needed.

These fields and startup role loading are implemented in the shared registry. The active reply entry records verified upstream revision/checksum metadata, not invented placeholder weights.

## Model selection and direct downloads

The canonical Models page retains the existing grouped manifest list/details layout and covers **Transcription (STT)** and **Reply (LLM)** together, without purpose tabs or a second standalone picker. **Downloads happen directly in the TUI process or during explicit server startup. There are no public or private model-management HTTP routes, operator APIs, download services, or management credentials.** The web neither downloads nor selects models; it uses the models already loaded by the server. Voice synthesis stays client-side and is not another server model slot.

- Read the existing local `models/manifest.toml` through `ModelManifest`/`ModelCatalog`, using the same `[[models]]` format and optional fields above. Preserve existing IDs/metadata, add license/quantization where needed, and derive available downloads and role-file details from these entries. The current reply group contains the pinned Qwen starter; other wireframe candidate names remain illustrative.
- Extend `scripts/download-model.sh` directly, reusing its checksum, resume, `.part` promotion, and per-model dispatcher. Keep current Whisper/Zipformer arguments, no-argument Whisper default, `--list`, `--help`, and experimental SeaLLMs consent behavior. The Qwen starter has a vetted ID/source/checksum; SeaLLMs-Audio remains download-only, not an automatically supported reply model.
- Reuse existing `ModelCatalog`/`DownloadTask` patterns. TUI Download directly launches that script with a validated model argument and the configured model root, exactly like today's STT downloader. Server startup flags invoke the same script if the explicitly selected files are missing. Use the registry's validated `download_id` rather than another TUI candidate allowlist; the script still enforces its pinned target allowlist. No download HTTP request or duplicate download implementation.
- Store artifacts under local `models/transcript/` or `models/reply/`. For this development workflow, run model management on the server's host/workspace so the files are available to its next startup. A TUI on another machine can inspect the web, but its local download does not magically install weights on the remote server.
- Show expected bytes, license/source, destination, and available disk space when readable before confirming network access. Use pinned allowlisted entries and checksum verification; no shell interpolation or arbitrary user-supplied URL execution.
- Show Missing, Downloading, Verifying, Downloaded, Failed, or Cancelled independently of runtime support and server readiness. A downloaded file is not automatically loaded. Runtime/active status comes from read-only server inspection when connected, not from assuming that local files are active.
- Keep one local download task at a time, with progress/logs inline below the right-hand model information and retained after completion/failure/cancellation. Enter on a missing row confirms network access; F7 cancels, and an explicit later Enter retries/resumes. Downloading can run while the web is used but may share bandwidth/CPU/disk. Downloading alone never changes active models or web history.
- Retain partial files for explicit retry/resume where supported and promote only verified artifacts. Unknown totals/progress stay unavailable. Exiting the TUI cancels its own child download, as the existing task does; valid partial data may be resumed later.
- Select a transcription/reply pair in local configuration for server startup. Display **Chosen for next start** separately from **Active on server**. Saving a choice does not hot-swap a running server or cause any web submission.
- Apply a changed server model pair through startup flags or an explicit server restart using those flags. The TUI displays the launch command; if a later launcher owns the server process, it can use ordinary process control, not a management API. Do not terminate an externally started server automatically.
- Clearly warn that restarting loses the in-memory web conversation, for either STT or reply changes. There is no live-switch/rollback promise. In-flight web work must be finished or deliberately cancelled before a restart; keep the current runtime untouched while merely selecting/downloading.
- Enter on an available STT row loads/prepares it only in standalone mode. Connected clients never load local models; available reply rows give server-startup guidance in either mode. Server-connected tests use whatever the server actually loaded; a local choice alone does not override that runtime or create another reply-model instance.
- Reply support remains a normal dependency with no feature-enable/build step. Unsupported server STT adapters are shown honestly; the existing standalone adapter-preparation workflow remains separate.

The Models screen works without a running server because its catalog, downloads, and startup choices are local. Server active/readiness badges show unavailable when disconnected. Opening the web or calling voice/readiness endpoints can never invoke this downloader.

### Server startup model flags

Explicit runtime model selection is available at startup as an alternative to using the TUI. Build both binaries first; only the existing STT adapter uses a Cargo feature:

```text
cargo build --release -p server -p reply-native --features server/whisper
./target/release/server --stt-model whisper-large-v3-turbo --reply-model qwen2.5-1.5b-instruct-q4-k-m
```

Each model-ID flag selects a pinned catalog entry for that purpose. If its files are missing, startup directly invokes the extended `scripts/download-model.sh` for only that explicitly selected model after source/checksum/license validation. It must not download the whole catalog or infer consent from a web request. Show progress/errors in server startup output and reuse verified files. Both the TUI and startup use the same script's per-model destination rules under `models/transcript/` and `models/reply/`.

Retain existing path-based `--model` behavior for compatibility. A missing arbitrary file path produces an error, not an inferred URL/download. Reject conflicting path/ID settings rather than silently choose one. Persist selections made through the TUI; explicit startup model flags override each saved slot and server output reports the effective choice.

TUI config adds `reply_role_files`, mapping model IDs to ordered absolute role paths. Dedicated `StartupChoices` retains `stt_model`/`reply_model` and adds optional `reply_prompt_files`, defaulting to empty for older files. Relative prompt paths resolve from the choices file directory; a non-empty list requires a reply-model selection. Repeated `--prompt-file` flags combine sources in flag order and conflict with legacy single-file `--system-prompt`. Explicit prompt flags override saved roles. Without them, saved roles apply only to a matching reply model ID; a different explicit model uses its own manifest default rather than inheriting old roles. Raw `--reply-path` requires explicit prompt flags and never inherits saved roles. Without an applicable saved/explicit role choice, a selected catalog reply model uses its manifest `system_prompt`.

Opening the web, calling a voice endpoint, or polling readiness must never start a download, model build, or activation. Web receives not-ready information and an instruction to ask the operator to use the TUI or startup flags.

## Incident role prompt and answering boundary

Use the checked-in [incident role prompt](../../pheme-va/roles/incident-reporting.txt) at `pheme-va/roles/incident-reporting.txt`. It is a checked-in instruction asset **loaded by reply startup**. Bind it through each incident reply model's registry `system_prompt`; share the file across reply models rather than duplicate equivalent text per weight file. A different role can use a different file on its reply entry when genuinely needed.

The role aligns with the existing Go and Rust `IncidentReport` concepts: `incident_type`, `location`, `severity`, `summary`, and `recommended_action`. It requires grounded facts, preserved uncertainty/negation, one clarification at a time, explicit corrections, concise spoken read-back, and no claims of saving, dispatch, retrieval, or execution. Missing details stay unknown. Recommended actions are proposals, not completed actions.

This voice-reply prompt returns natural-language text, not the existing incident endpoint's JSON schema. Structured incident extraction remains behind its analyzer contract and validation; do not break `POST /api/v1/incidents/analyze` or pretend its placeholder is already a model-backed workflow. The web voice loop now uses these bindings; this does not implement confirmed incident reports or replace structured analysis.

```text
Audio → Transcriber (Whisper/Zipformer) → transcript review
                                            ↓ approved text
                             ConversationModel (separate reply LLM)
                             + registry-bound incident role prompt
                                            ↓
                                   answer text → web TTS
```

**Never route approved text back into whisper.cpp to obtain an answer.** Whisper/Zipformer entries are transcription models; their optional dictionary/snippet hints are ASR vocabulary context, not an incident-assistant system prompt. Do not attach this role file to a transcription entry, use `LlmTextCleaner` as the answer path, or fall back to STT if reply model/prompt readiness fails.

Role loading is part of reply startup: resolve the manifest-relative default or chosen ordered sources, validate non-empty UTF-8 regular files, load once, and include the combined text as a system message. Preserve each source's exact text and join with exact `\n\n` separators in selected order. Allow 1–32 files and at most `MAX_PROMPT_CHARS` (16,384 Unicode scalar values) in the combined prompt, including separators. The existing default 12,000-character aggregate role/history/approved-question budget still applies; startup rejects a role that consumes the entire budget. A missing/unreadable/invalid role makes answering unavailable with a clear error, never a generic fallback. Trusted startup overrides follow the precedence above and report ordered source paths/hash. Web/user text cannot choose file paths or rewrite system instructions.

For a web reply, build messages from the loaded role, bounded successful web history, and exact approved question. Isolated tests use the selected reply model's role with fresh test context, never web history. Adapter-owned chat templates encode these separate message roles; raw user text must not become part of the system instruction.

Expose role sources and validity in TUI model details. `o` opens the reply-only Roles picker; `p` previews the combined text and exact-byte UTF-8 SHA-256. Inspection's role name/path identifies the first source, while role/turn hashes cover the entire combination. Record the combined hash in metadata without recording sensitive prompt contents by default. Role selection and file edits take effect only on server restart. No live reload or browser model/prompt-management endpoint is implemented.

Prompts guide model behavior, not authorization. Pheme workflow code must still enforce validation and future revision-bound confirmation; this slice has no save/finalize/tool capability for the model to execute. No profile/session framework or other use case is introduced.

## Web request loop and transcript review

The web records a complete clip, starts a voice-turn request, and consumes that request's streaming response. The response starts with a `turn_id`, then carries transcription status and the processed transcript. It remains open while waiting for explicit approval, with heartbeats and a bounded review timeout.

- Web opens an editor seeded with the processed transcript. TUI inspection exposes that same pending turn for optional correction/submission.
- Keep raw text separately for diagnostics, never as a substitute for the approved question.
- Web/TUI keystrokes stay local. There is no collaborative editor or Save-draft protocol.
- Either interface can submit the exact edited text against the pending `turn_id`. Blank/overlong text is rejected before generation.
- Atomically check the turn identity, awaiting-review state, and runtime availability before accepting generation. A stale TUI edit cannot target a newer web question, and competing submissions cannot start two generations. A busy reply leaves the web turn awaiting review for an explicit retry.
- The first accepted submission freezes that turn's question. An identical repeat returns existing status; a different second submission conflicts. Editing an already-started or completed question requires a new web turn, not rewriting history.
- Include the accepted question in the web response. If TUI corrected it, the web displays that corrected wording and exits review instead of keeping a different local question.
- Approval sends no reply to a global feed. Generated text goes to the web's original response; TUI reads the associated state through inspection.
- Silence, insufficient speech, empty transcripts, and transcription failures never start answering. Web can abandon/cancel a pending turn and re-record. Text-only input can also start a reviewable web turn.

The original response stays open across transcription, review, and answering specifically so TUI approval can complete the web operation without the browser subscribing to a separate broadcast channel. Completed-turn STT is intentional; streaming the reply does not require streaming recognition.

## Direct reply streaming and TUI inspection

Use Server-Sent Events (SSE) as the format of the **web turn's direct HTTP response**, not an all-client subscription feed. Its events include `turn.created`, `transcript.ready`, `question.approved`, `reply.started`, `reply.delta`, and terminal completed/failed/cancelled status, correlated with that turn.

Emit reply text during actual generation; do not split a finished answer into fake streamed chunks. Deltas are valid Unicode text without runtime stop/control tokens. Final completion includes authoritative full answer text, allowing the web to replace its provisional buffer.

TUI inspection is ordinary snapshot polling in a background worker, for example a configurable few times per second. The snapshot includes web history, current turn ID, transcript, approved question, accumulated partial reply, final answer, status, readiness, and relevant timings/errors. Polling replaces the rendered state rather than appending the same partial text repeatedly. A TUI opened halfway through generation can immediately inspect the answer so far. Test transcript/reply content is excluded.

No global event bus, subscriber registry, event cursors, or historical replay store is required. The existing `/v1/metrics/batches` drain is not an inspection/voice transport.

Go forwards each request-scoped stream without buffering the full answer and flushes promptly. Use heartbeats and HTTP/proxy timeouts that allow the bounded review wait. Browser `fetch` consumes the POST response with a tested SSE parser; network chunks can split Unicode or event frames.

Disconnecting the TUI inspection view has no effect on a web turn. Closing an isolated TUI test stream cancels only that test through its inference cancellation signal. If the web transport drops, inspect that turn's current status/result rather than automatically starting generation again. Bound abandoned review waits. Already-accepted inference can finish and retain its result for recovery; a slow/disconnected browser must not block inference or create unlimited buffering. Restored results never trigger automatic speech.

Cancellation must reach the native inference loop, not merely close an HTTP response or abandon a waiting task. Keep cancellation, review timeout, shutdown, and stale-result handling explicit.

## Implemented voice API

Go exposes corresponding public routes under `/api/v1/voice/` through its Pheme client. The routes below are implemented; see the API contract for limits/errors. `:turn_id` is documentation notation; use the router's actual parameter syntax during implementation.

| Pheme route                            | Purpose                                                                                            |
| -------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `POST /v1/voice/turns`                 | Start a web-owned WAV/text turn; return its direct stream through transcription, review, and reply |
| `POST /v1/voice/turns/:turn_id/submit` | Web or TUI approves edited text for that pending web turn; acknowledge promptly                    |
| `GET /v1/voice/turns/:turn_id`         | Read current status/result for web recovery without replay or regeneration                         |
| `POST /v1/voice/turns/:turn_id/cancel` | Cancel the identified web turn                                                                     |
| `POST /v1/voice/reset`                 | Explicitly clear web conversation history after settling active work                               |
| `GET /v1/voice/inspect`                | TUI reads current web conversation/turn snapshots; excludes independent test content               |
| `POST /v1/voice/test/reply`            | Run an isolated reply test; stream its answer only to the caller                                   |

There are no model catalog/download/select/cancel HTTP endpoints. The TUI reads the local manifest and invokes the downloader directly; server startup resolves its explicit model flags locally. The voice routes above remain solely for the conversation, inspection, and isolated inference tests.

The submit body can be as small as:

```json
{
  "text": "The web question after I corrected its transcript"
}
```

The URL identifies the web turn. Submission returns its known status promptly; generated text continues through the original web response, not through the TUI submit response. Bound retained turn/results and duplicate-request records. Known identical creation/submission retries must not start another inference; conflicting reuse returns a conflict. If a result is no longer retained or the server restarted, report that explicitly rather than automatically rerun the question.

TUI audio tests use existing stateless `/v1/transcribe`, then the isolated test-reply route if needed. Existing `/v1/transcribe`, `/v1/analyze`, health/readiness, metrics, and Go incident/benchmark contracts remain unchanged. These ordinary stateless/benchmark operations do not overwrite web inspection state.

Keep HTTP concerns in Go handlers, delegation in services, and integration/stream forwarding in the Pheme client. Use `context.Context`, constructor injection, sanitized errors, and existing mocks. Do not overload `IncidentAnalyzer.Analyze` with this conversation loop. Neither Go nor the TUI owns another copy of authoritative web workflow rules. Model selection/downloads are local CLI/TUI workflows outside this HTTP delegation. Neither Go nor a private Pheme route initiates them.

## TUI workspace and wireframes

The existing `ratatui` layout remains an engineering console, not a new UI framework. One canonical Models page reuses grouped manifest rows/details and inline download progress in both modes; the test bench and telemetry retain their established patterns.

### Navigation and persistent status

- Main views: **w Web**, **b Tests/back**, **m Models**, and **t Telemetry**. Numeric 1–4 never selects main views; only Telemetry uses 1–4 for Overview/Metrics/Runs/Logs, with Esc returning to the previous view. Focused text input retains navigation letters/digits. F6 explicitly approves editor/test text for reply; F8 stops local playback.
- Always show the connection target/mode, connection state, active transcription model, active reply model, and each runtime's readiness. Merely highlighting a catalog model must not change these active badges.
- Distinguish **Server-connected inference** from **Local standalone** visibly. Models always reads/downloads local files directly. Show the local model root, chosen startup pair, and separately the server's reported active pair; a connected server is not a remotely managed download target. Never silently cross that boundary.
- Keep the existing 80x24 minimum. At narrow supported sizes, stack panels or show list/details separately; long history, paths, catalogs, and logs scroll. Wider terminals can show side-by-side panels.
- Footer shortcuts depend on focused view. Navigation shortcuts do not fire while editing text, and destructive/model-changing actions require confirmation.

Wireframes show layout intent; implemented screens follow the existing Ratatui patterns. Reply model names, paths, progress, and timings below are illustrative; actual entries come from the pinned catalog.

### Web inspector

```text
+------------------------------------------------------------------------+
| PHEME VA   SERVER: localhost   CONNECTED                                |
| STT: whisper-large-v3-turbo [ready]  Reply: not selected                 |
| [w Web]   b Tests   m Models   t Telemetry                               |
+--------------------------------------+---------------------------------+
| WEB CONVERSATION                     | CURRENT WEB TURN                |
|                                      | ID: turn-123                    |
| User: earlier approved question      | State: awaiting review          |
| Reply: earlier completed answer      |                                 |
|                                      | Transcript:                     |
|                                      | There is smoke at the east gate.|
|                                      |                                 |
|                                      | Reply: waiting for approval     |
+--------------------------------------+---------------------------------+
| STT timing: available on completion  Reply timing: not started          |
| Reply unavailable: open Models to download/select one.                 |
+------------------------------------------------------------------------+
| e Edit web transcript   p Replay completed answer   q Quit             |
+------------------------------------------------------------------------+
```

Poll only web inspection state, not a combined test/conversation feed. A reply streams to the web while the TUI periodically replaces its partial-answer panel from inspection. Reply readiness gates Submit, not the ability to observe/edit a transcript. The `e` action is enabled only for a pending reviewable web turn; Enter on the inspector never automatically submits it.

### Web transcript editor

```text
+------------------------------------------------------------------------+
| EDIT WEB TURN: turn-123                         Unsaved local changes   |
+------------------------------------------------------------------------+
| Original: There is smoke at the east gate.                              |
|                                                                        |
| Approved question:                                                     |
| There is smoke at the west gate.                                       |
|                                                                        |
| This approves the WEB turn. The answer returns to the WEB request.      |
| It is not an independent TUI test.                                      |
+------------------------------------------------------------------------+
| F6 Submit to reply model   Esc Close editor   arrows Move cursor        |
+------------------------------------------------------------------------+
```

Bind the editor to its turn ID. Polling must not clobber dirty text or retarget it to a newer turn. Support insertion/deletion, Unicode and wrapping; preserve local text on a failed/late submission. After server acceptance, show the authoritative approved wording as read-only. The web remains playback owner.

### Independent test bench

```text
+------------------------------------------------------------------------+
| PHEME VA   SERVER TESTS - ISOLATED FROM WEB                              |
| STT: active server model   Reply: active server model                    |
| w Web   [b Tests]   m Models   t Telemetry                               |
+-------------------------+----------------------------------------------+
| SOURCE                  | REVIEW TRANSCRIPT / TYPED QUESTION           |
| l Microphone            |                                              |
| f WAV file              | Editable test text appears here.             |
| i Typed text            |                                              |
|                         +----------------------------------------------+
| Test stage: review      | TEST REPLY                                   |
| Voice playback: off     |                                              |
|                         | Streamed test result appears here only.      |
+-------------------------+----------------------------------------------+
| Test timings / run ID / errors - no updates sent to web                 |
+------------------------------------------------------------------------+
| F6 Run reply test   r Retry   p Replay   m Models   q Quit               |
+------------------------------------------------------------------------+
```

Retain microphone/WAV recording, stop limits, file browsing, raw-transcript diagnostics, and isolated reply tests. Offer STT-only, typed-reply, or STT-review-reply testing without implicitly submitting to the web turn endpoint. Server tests use the active server models; `m` opens local model files/startup choices, not a per-test runtime swap. A next-start choice does not change these active models. Preserve the existing standalone local STT bench as a separate mode.

Network polling, commands, local child downloads, test streams, and playback never block terminal input/rendering. Server-connected views do not load another local model. Quitting stops TUI observation/tests and cancels its local download child; it does not stop web work or an independently running server. Observed web content is not silently saved as a local STT run report.

### Unified Models catalog: transcription and reply

```text
+------------------------------------------------------------------------+
| PHEME VA   MODEL FILES: LOCAL   Server: inspection/inference only        |
| STT: whisper-large-v3-turbo [ready]  Reply: not selected                 |
| w Web   b Tests   [m Models]   t Telemetry                               |
+------------------------------------------------------------------------+
| Search all manifest entries:                                           |
+----------------------------------+-------------------------------------+
| VOICE & TRANSCRIPTION            | SELECTED MODEL INFO                 |
|   whisper-large-v3-turbo Available| Purpose: Reply / Runtime: llama.cpp |
|   zipformer-small       Missing  | Qwen2.5-1.5B-Instruct Q4_K_M         |
|   zipformer-medium      Missing  | Size / license: 1.12 GB / Apache-2.0 |
|   zipformer-large       Missing  | Source + revision: pinned catalog   |
|                                  | File: models/reply/<Qwen GGUF>      |
| REASONING & REPLY                | Chosen next start: no               |
| > qwen2.5-1.5b...    Downloading  | Role: roles/incident-reporting.txt  |
|                                  +-------------------------------------+
| All rows come from the manifest. | LOCAL DOWNLOAD (retained inline)    |
| Highlight is not activation.     | Downloading [########------] 40%    |
|                                  | Bytes / verification / child output |
|                                  | Active reply: unchanged             |
+----------------------------------+-------------------------------------+
| arrows/jk Rows   PgUp/PgDn Details   / Search   Enter Download/load      |
| s Choose   o Roles   p Combined preview   g Command   r Rescan           |
| F7 Cancel download   F8 Stop playback   b Tests/back                    |
+------------------------------------------------------------------------+
```

This is the same canonical grouped list/details layout in standalone and connected modes, not two model screens. All entries are discovered through the manifest; `/` searches across both purpose groups. Arrows/`j`/`k` move rows; PgUp/PgDn scroll details/settings/source/checksum. No purpose-tab or left/right purpose filtering is required. Distinguish highlighted, chosen-for-startup, local artifact state and reported-active entries. ASR entries show no conversation role; `p` previews the reply entry's default/saved combined text and hash.

Enter on a missing row confirms destination/size/license/source and network access, then starts `download-model.sh` directly. Enter on an available STT row loads/prepares it only in standalone mode; connected mode never loads local weights. Enter on an available reply row gives server-startup guidance in either mode. `s` hashes/verifies the supported artifact bundle and confirms its next-start choice, without calling/replacing the running server. `g` renders the launch command; `r` rescans local artifacts. Unsupported rows explain why they cannot be selected for inference; no reply feature build or management credential is involved.

Download progress stays **inline below the right-hand model information**, with terminal logs/status retained after completion, failure or cancellation. Render Downloading, Verifying, Downloaded, Failed, or Cancelled from the local child task. Percent/throughput/ETA appear only when reliable; otherwise show bytes and an indeterminate gauge. A successful script exit/checksum refreshes the catalog; activation still needs standalone Enter or confirmed `s` for next startup. There is no separate download-progress screen. Leaving Models keeps the task running; F7 cancels the download while on Models, quitting cancels the child, and a later explicit Enter can resume a valid `.part` file. Cancellation cleans up downloader subprocesses. Prevent TUI/server-startup writers from modifying the same partial artifact concurrently.

### Reply-only Roles picker

```text
+------------------------------------------------------------------------+
| ROLES: qwen2.5-1.5b-instruct-q4-k-m       Applies at next server start    |
+------------------------------------------------------------------------+
| Browse: pheme-va/roles/ (resolved from manifest default role)            |
| > [1] incident-reporting.txt                                           |
|                                                                        |
| Other local .txt files appear here when present.                        |
| Space adds/removes sources in selection order; a adds a local path.     |
| Selected sources: 1    Combined text/hash: p from Models after save     |
| Running server role: unchanged                                         |
+------------------------------------------------------------------------+
| arrows/jk Browse   Space Toggle   a Add path   Enter Save   Esc Cancel   |
+------------------------------------------------------------------------+
```

`o` is reply-only. Browse the directory containing the entry's manifest default role; the checked-in default is `roles/incident-reporting.txt`. Space toggles a file, appending new selections to the ordered source list; `a` adds an arbitrary trusted local `.txt` path. Enter validates the whole selection and saves absolute paths in `reply_role_files[model_id]`; Esc cancels. Errors preserve the prior configuration. Sources combine with exact `\n\n` separators within the file/character and aggregate request limits above; `p` previews the saved/default combined text/hash. Saving Roles does not load a model or change active instructions.

### Choose models for server startup

```text
+------------------------------------------------------------------------+
| SAVE SERVER STARTUP MODEL CHOICES                                       |
+------------------------------------------------------------------------+
| Chosen transcription: whisper-large-v3-turbo                            |
| Chosen reply:         Reply candidate A                                 |
| Reply roles: roles/incident-reporting.txt (default or saved order)       |
| Running server:       unchanged                                        |
|                                                                        |
| Save these choices locally; show the matching server startup flags.     |
| Start/restart the server to load them. No HTTP management call.          |
| A restart loses in-memory web history, for either model change.         |
| Finish or deliberately cancel active web work before restarting.       |
|                                                                        |
| This does not stop a server, submit a web turn, or run a test.            |
+------------------------------------------------------------------------+
| y Save choices   g Show startup command   Esc Cancel                    |
+------------------------------------------------------------------------+
```

`s` hashes/verifies artifacts before asking to save the next-start choice. Render the server command with the selected transcription/reply IDs and repeated `--prompt-file` flags for saved role sources in order. Current server badges remain unchanged until read-only inspection reports the newly started server's ready models. Startup shows loading/prewarming/errors in its terminal output. No hot-swap, automatic remote restart, or preserved-history rollback is implied. Missing/failed downloads do not change the running server; a failed restart is a startup failure, not a live-switch transaction.

### Telemetry, layout, and state boundaries

Preserve Overview/Metrics/Runs/Logs with existing filtering, run reports, and unavailable-value behavior. Add local download/verification/startup-choice status to TUI logs and metadata; actual model-load status comes from server startup/inspection. Do not invent inference measurements during a download. Label web inspection, isolated tests, and local model-file operations distinctly.

Implement screens with existing Ratatui lists/tables, text panels, tabs, and gauges. Extract editor/model-action state from drawing for deterministic tests. TUI state keeps one local grouped catalog/search, highlighted model ID, selected startup pair, per-model ordered role selections, confirmation, inline retained local download state, and dirty web editor buffer. Read-only server inspection remains authoritative for running model IDs/readiness and the web turn. Correlate async results with the local task or web turn, never whichever row happens to be highlighted now.

## Web voice console

Build the voice view with existing React/TypeScript, routing, Tailwind, shadcn components, and Lucide icons. Keep the benchmark dashboard requirement intact.

- Provide microphone permission/start/stop, transcript review, Submit, request status, streamed answer, replay/Stop voice, Stop generation, and web-context reset.
- Start and consume the web-owned request stream; do not subscribe to a global feed or receive independent TUI test updates.
- Display current model names/readiness only. No model download/picker/startup controls or model-management endpoints exist for the web; missing-model voice requests report not-ready and never invoke a downloader.
- When TUI approves a correction, show the accepted wording from this turn's response and disable conflicting local submission.
- Encode supported WAV from browser PCM. Do not send arbitrary `MediaRecorder` WebM/Opus to a WAV-only pipeline. Pheme remains authoritative for normalization/downmixing/resampling.
- Enforce configured recording/input limits. Microphone access requires localhost or another secure context; LAN/mobile access normally requires HTTPS.
- Render model text safely, not as executable HTML. Preserve answer text if TTS fails.
- Recover transport loss by reading this turn's status/result without automatic resubmission, replayed speech, or browser content persistence.

## Client-side voice playback

The server returns text, not speaker playback. Web uses its own speech adapter; TUI tests/replay can use a local synthesizer.

- Stream text during generation, then speak only a completed validated answer. Text streaming alone does not start speech earlier.
- **A web-owned turn is spoken by the web even if TUI edited/submitted its text.** Playback ownership follows the original request, not whoever clicked Submit.
- TUI web inspection stays silent by default. Optional explicit local replay does not regenerate the answer or redirect web playback.
- An independent TUI test can speak locally if enabled, but never triggers web playback.
- Restored history/status/result never auto-speaks. Only a live completion of the web's own active response is eligible; recovery requires deliberate Replay.
- Run TUI synthesis off the UI thread with responsive Stop. Command adapters pass text via stdin/arguments without shell interpolation and handle missing executables/timeouts safely.
- Browser voices are not necessarily offline. Verify execution location and do not silently use remote TTS by default.
- TTS failure does not lose displayed text, undo history, or invoke the reply model again. Stop voice is local; Stop generation targets the relevant backend operation. Stop playback before recording in that client.

Sentence-streamed speech, acoustic echo cancellation, and full-duplex interruption are outside this implementation.

## Metrics, privacy, and limits

Use existing metrics infrastructure to separate transcription, human review, first-text/full-generation latency, model load time, and client synthesis/playback outcomes. Label web-turn versus isolated-test runs so their timings/content are not mistaken for the same conversation. Keep server inference CPU/RAM separate from client resources; unavailable values stay unavailable and CPU utilization is not measured power.

Web response streams and TUI inspection intentionally contain web questions/answers. Default logs/metrics do not. Keep history/results bounded in memory, release audio after processing, and do not silently persist inspected web content into TUI run history or browser storage. Independent test content must not appear in web history, response streams, or inspection snapshots.

Inspection and its edit/submit controls are developer capabilities, not public access to everybody's speech. Bind to loopback by default, protect inspection/mutation endpoints, and require suitable access controls, allowed origins, and transport protection for LAN exposure. Correlation IDs are not credentials and CORS is not authentication. Rust remains behind Go in the normal setup.

This remains one development web conversation, not a multi-user system. No session registry, draft revision system, database, distributed queue, replay store, or extra service is required. Incident/session persistence is separately planned work.

## Verification

Use local fake transcriber, streaming reply model, and synthesizer implementations to cover:

- no answering before approval; the model receives exactly the edited text from web or TUI;
- TUI approves a web transcript and the corrected question/reply reaches the original web response, not a TUI-owned stream;
- independent TUI transcription/reply tests create no web messages/history/inspection/playback updates and receive no web-history context;
- shared model weights are reused while per-call inference/KV context remains isolated;
- only one pending web turn/inference operation as configured, busy handling, competing submissions, known retries, stale editor turn IDs, and frozen approved questions;
- successful web replies commit one pair; failed/cancelled replies leave no partial successful history;
- TUI attaches mid-generation and replaces polled snapshots without duplicate text or dirty-buffer loss;
- web transport loss/status recovery, bounded abandoned review waits, slow clients, real cancellation, reset, and no stale output after reset;
- WAV/silence/empty-input handling, Unicode/frame-split SSE parsing, browser/TUI TTS failures, and no automatic speech on recovery;
- models load at server startup and are reused for ordinary turns, no additional runtime loads in server-connected clients, and model paths use `models/transcript/` and `models/reply/`;
- one grouped Models page in both modes, cross-purpose search, row/details scrolling, selected-versus-active badges, 80x24 layout, Enter's missing/standalone-STT/reply behavior, explicit download/startup-choice confirmations, alphabetic main navigation and Telemetry-only numeric tabs, and editor/navigation focus handling;
- direct `DownloadTask` script invocation with validated model arguments, inline right-hand progress retained after completion, checksum-failure handling, partial resume, F7 process cancellation, and no concurrent partial-file writers, using fake script fixtures rather than network/weights;
- existing `[[models]]` registry discovery without a separate hard-coded TUI list, legacy ASR defaults/aliases, `download_id`/script consistency, complete download metadata, missing-artifact bundles, purpose-specific destinations, and preserved script arguments/consent;
- per-reply `system_prompt` resolution, Roles browsing/toggle/add/save/cancel, per-model absolute-path persistence, ordered exact `\n\n` composition and combined preview/hash, 32-file/character/aggregate limits, missing/empty/invalid prompt errors, additive choices-file-relative `reply_prompt_files`, repeated prompt flags and conflict/override precedence, and role separation from user content;
- transcript models receive ASR hints only and approved questions go to the reply adapter, never Whisper/Zipformer or the cleanup path; isolated tests inherit the reply role but no web history;
- no HTTP route or web/readiness request triggers download/selection; local saved choices do not alter an already running server, and startup flags are explicit.

Manual acceptance:

- Open TUI Models before starting the backend. Verify one grouped manifest catalog covers Transcription/Reply, `/` searches both groups, arrows/`j`/`k` move rows and PgUp/PgDn scroll details. Confirm an Enter download, watch progress inline under the right-hand info, and verify terminal status remains visible. Use `s` to hash/confirm the pair and compare local/chosen versus active status. Incomplete artifacts or missing reply-role files are clearly unavailable, not silently replaced.
- On a reply row, open `o`; verify the default incident role directory/file, Space toggles, `a` adds a trusted local `.txt` path, Enter validates/saves, and Esc cancels. Preview the ordered combination/hash with `p`, inspect `g` for repeated prompt flags, and verify roles take effect only on a matching-model server restart. Check different-model selection, raw-path explicit prompts, legacy configs and choices-file-relative sources without live reload.
- Verify `w`/`b`/`m`/`t` main navigation; only Telemetry uses numeric 1–4. F6 explicitly approves edited reply input, and F8 stops only local playback. Available STT Enter loads/prepares only standalone; connected STT and available reply rows give startup guidance.
- Start the backend with the displayed model flags, open the web console, then attach the TUI inspection view without any session setup. Verify actual active models/readiness are distinct from locally highlighted/chosen entries.
- Record on the web. Verify the returned transcript also appears in TUI inspection and no answer starts automatically.
- Correct a word in the TUI and submit that pending web turn. Verify the web displays the corrected question, receives the streamed reply through its existing request, and speaks it. TUI displays the reply through inspection without automatic playback.
- Run an independent TUI audio/reply test. Verify it stays in the TUI test view and produces no web question, answer, history change, or speech.
- Exercise web-side approval, competing/stale submissions, reconnect during generation, cancellation, reset, and a TTS failure. No duplicate inference or partial successful history should appear.
- Download another candidate directly in the TUI while inspecting the web; verify no active model/history changes. Cancel/retry the local script, save a different next-start choice, and verify it takes effect only after an explicit server restart. Web offers no download controls and requests cannot trigger one.

Run relevant checks during implementation:

```bash
# pheme-va/
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# api/
gofmt -l .
go vet ./...
go test ./...

# web/
npm run lint
npm run build
```

Real-model/audio/TTS checks remain opt-in and record exact runtime settings and measured results. Ordinary CI uses fakes/small fixtures without downloading weights. Update model/setup/API documentation when the implementation ships; the implementation status above distinguishes automated checks from still-unverified real-model/browser acceptance.
