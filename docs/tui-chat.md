# TUI chat and conversation measurements

Chat and isolated Tests call `va_runtime` directly in Rust. They need no Go or HTTP service. From `pheme-va/`, build the CLI and its persistent reply worker:

```bash
cargo build --release -p cli -p reply-native --features cli/whisper
./target/release/cli tui
```

The TUI opens Models with the saved voice and reply selections visible in the header and marked **[SELECTED]** in the list. **Enter** prepares and selects the highlighted model, remembering the choice automatically. Missing files first request download confirmation. A missing speech adapter compiles or reuses its cached build; activating a different reply model automatically relaunches the TUI. Selecting an already active model keeps it loaded. **b** opens Chat only after both selected models are ready. Subsequent launches restore and load the saved pair without reselecting them. The existing `selected_stt_model` and `server_reply_model` configuration keys are retained. Ordered per-model role files are loaded once at startup.

```text
PHEME VA / MODELS
Voice: whisper-large-v3-turbo [ready]
Reply: qwen2.5-1.5b-instruct-q4-k-m [ready]
┌ MODELS ─────────────────────────────┐ ┌ MODEL DETAILS ──────────────────────┐
│ Voice & transcription               │ │ zipformer-small                    │
│   [SELECTED] whisper-large-v3-turbo  │ │ Selection: not selected            │
│ > zipformer-small                   │ │ Adapter: prepare on selection      │
│   zipformer-medium                  │ │                                    │
│ Reasoning & reply                   │ │ Enter prepares and selects model.  │
│   [SELECTED] qwen2.5-1.5b…           │ │ Choices are remembered.            │
└─────────────────────────────────────┘ └────────────────────────────────────┘
[b] Chat  [t] Telemetry  [w] Web  [↑/↓] Browse  [Enter] Select
```

Adapter caches are retained independently of the chosen weights. A fresh launcher can reuse a combined Whisper/Zipformer build when it contains every required adapter and accelerator; exact builds are preferred. Source, dependencies, compiler, build configuration and native runtime integrity determine reuse. Changing samples, runtime roles or documentation does not require recompilation. Version 4 cache metadata lives in the existing `target/tui-adapters/cache-v2/` directory; earlier generations remain on disk but must be rebuilt before reuse.

```text
TUI Chat / Tests ── Rust calls + bounded channels ──→ va_runtime → core + adapters
Web → Go → HTTP host ──────────────────────────────→ va_runtime → core + adapters
```

These hosts share code. Separate TUI and server processes load separate weights and keep separate conversations. `tui --server-url http://127.0.0.1:8080` enables only the optional Web inspector/approval page. Local snapshots recover from event-channel lag without regenerating a reply; HTTP/SSE remain the transport for Web clients.

Local metrics follow saved TUI collection settings; resource sampling runs at 250 ms when enabled. Measurement flags and unavailable values remain visible when collection is disabled.

The Source panel keeps microphone and WAV selection. While a complete clip is being normalized, gated or transcribed, Chat shows the STT model name and animated dots. The returned transcript immediately enters review; reasoning receives only the wording explicitly confirmed with Enter. After approval, the reply model shows dots until its first real text, then streams its answer. Repeat with microphone, WAV or typed input.

| State | Keys |
| --- | --- |
| Draft review | Enter confirm/send; e edit; x discard |
| Editing | Enter confirm/send; Alt+Enter newline; Esc review with edits retained; arrows/Home/End/Backspace/Delete edit |
| Idle chat | l microphone; f WAV picker; n next file; r retry source; i type |
| Recording | Enter stop/transcribe; x discard |
| Processing | x cancel; native work must settle before another inference |
| Conversation | c finish/new; d finish; b return to Chat |
| Other pages | m Models; t Telemetry; w Web; s isolated local tests from Chat |
| Playback | p replay; v toggle automatic speech; z stop speech |

Bracketed actions appear in the footer. Transcript review keeps its workflow active, but still offers **Enter** to confirm/send and **e** to edit; native compute availability is checked by the runtime at submission. Letters and digits are literal text while editing. Shortcuts have two spaces between items, start at the left, and wrap whole items; there are no function-key actions. Multiline paste preserves newlines. New/Finish cannot discard an unsent draft or bypass unsettled inference.

A wide screen during review looks like this (example content, measurements illustrative):

```text
PHEME VA / CHAT
Active STT: whisper-large-v3-turbo | Reply: qwen2.5-1.5b-instruct-q4-k-m
LOCAL CHAT / shared Rust runtime
┌ SOURCE ───────────────────┐ ┌ CHAT / Conversation 2026-10-08 09:30 UTC ─────────┐
│ INPUT SOURCE              │ │                                  You             │
│ [l] Live microphone       │ │                 ┌──────────────────────────────┐ │
│ [f] Select WAV file       │ │                 │ There is smoke downstairs.   │ │
│                           │ │                 └──────────────────────────────┘ │
│ Folder: samples           │ │ qwen2.5-1.5b-instruct-q4-k-m                      │
│ 1 WAV file(s)             │ │ ┌──────────────────────────────────────────────┐ │
│                           │ │ │ Is anyone in immediate danger?               │ │
│                           │ │ └──────────────────────────────────────────────┘ │
│                           │ ├ EDIT YOUR MESSAGE / NOT SENT ───────────────────┤
│                           │ │ Everyone has left the building.                │
└───────────────────────────┘ └──────────────────────────────────────────────────┘
┌ TURN / RUN DETAILS ───────────────────────────────────────────────────────────┐
│ Stage: awaiting review · Source: live microphone                               │
│ STT run: stt-conv-... · Reply run: —                                            │
└───────────────────────────────────────────────────────────────────────────────┘
┌ LIVE METRICS ─────────────────────────────────────────────────────────────────┐
│ Elapsed 1.4s · First text unavailable · STT 1.40s · Reply unavailable            │
│ Process CPU 42% · Process RAM 1900000000 bytes · Power unavailable              │
└───────────────────────────────────────────────────────────────────────────────┘
[b] Chat  [m] Models  [t] Telemetry  [w] Web  [c] New  [d] Finish
[Enter] Confirm & send  [e] Edit  [x] Discard  [s] Tests  [p] Replay
[v] Auto voice  [z] Stop voice  [j/k] Scroll  [End] Latest  [?] Help  [q] Quit
```

The **MESSAGE** composer is always visible. **i** focuses it; Enter sends the exact typed text in one admission, with no transcription or human-review stage. During generation, **i** lets you draft the next message; sending waits until native work settles. The model indicator appears in the chat history:

```text
whisper-large-v3-turbo                  qwen2.5-1.5b-instruct-q4-k-m
┌ ...  Transcribing audio ┐            ┌ ...  Generating reply ┐
Stage: normalization / gate / STT      Stage: generating
```

At 80×24 the Source controls stack above Chat; all footer actions remain visible. Message wrapping and cursor placement use terminal cells/graphemes, including wide Unicode characters.

Press t, then 3, to inspect grouped conversations:

```text
PHEME VA / METRICS & LOGS
[1] Overview  [2] Metrics  [3] Conversations  [4] Logs
┌ CONVERSATIONS / ENTER TO OPEN ────────────────────────────────────────────────┐
│ Title                                  Status       Turns       Runs         │
│ > Smoke downstairs and safe evacuation finished         2          4         │
│   Conversation 2026-10-08 10:15 UTC     active           1          1         │
│   Legacy standalone runs               archived         0          2         │
└──────────────────────────────────────────────────────────────────────────────┘
[1-4] Tab  [j/k] Select  [Enter] Open conversation  [/] Search
[c] Clear history  [Esc] Back
```

Enter opens the conversation overview: turn outcomes, audio duration, separate STT/reasoning/title time, human review time, wall time and every collected metric series. Rows retain source/scope/unit, latest/min/mean/max, total sample count and unavailable count. **r** opens stage runs with their individual metadata, raw transcript, submitted text, reply, errors and measurements; **h** opens saved chat. Enter on a metric opens numeric history and retained raw events, preserving unavailable gaps and signed values.

An audio turn creates separate STT and reasoning runs linked to one turn/conversation; a typed turn creates only reasoning. Finish/New asks the reply model for a brief title using completed exchanges. Title generation has its own maintenance run and is excluded from STT/reasoning totals. If it fails, the UTC placeholder remains.

Resource samples distinguish the hosting process, system and device scopes and measurement sources. Reply-child CPU/RAM, power and energy remain unavailable when no adapter measures them. Nested timings and cumulative counters are not summed into invented totals.

History version 2 reads version 1 runs into a Legacy group without invented chat turns. Archives retain at most 100 stage runs/legacy reports, 10,000 raw events per run and 64 MiB; oldest closed conversations are evicted as a whole, active conversations are protected, and each conversation stops at 32 turns. Aggregates survive raw-event truncation. Saved histories are read-only; they do not resume authoritative runtime sessions after restart. Missing or failed models keep Chat locked; the separate Tests view still supports whichever local adapter is ready.

See [API routes and measurement fields](api/voice.md#console-conversations). Live microphone/TTS behavior and real-model output quality still need device acceptance testing.
