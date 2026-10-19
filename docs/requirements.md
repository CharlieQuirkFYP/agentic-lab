# Requirements and Scope

This document records the current KLASS direction and the resulting implementation plan. Planned capabilities are not claims about the current code. See [architecture](architecture.md) for the boundary and implementation snapshot, [roadmap](roadmap.md) for delivery steps, and [benchmark methodology](benchmark-methodology.md) for evaluation.

## Confirmed direction

- The team builds the use-case solution itself, using pretrained models and libraries where appropriate. Existing KLASS solutions and adapters are excluded.
- Voice-Based Incident Reporting is the active use case, ideally speech-to-speech with agentic clarification and a human in the loop.
- Use Case 3, vision/licence-plate monitoring, is removed. Interview understanding is outside the current development focus but was not explicitly removed.
- Efficient local deployment is a core research objective. Power consumption must be measured alongside quality and performance.
- Whisper/whisper.cpp is the current speech-recognition baseline, not a benchmark-validated device configuration. A llama.cpp-compatible language-model adapter remains future work.
- Local speech nuances should inform model/dataset evaluation and adaptation. MERaLiON is optional, not mandated.
- A mobile phone is the preferred eventual platform. KLASS may supply an iOS device or NVIDIA Jetson; selection and specifications are pending.
- Development should proceed without waiting for hardware.

## Service boundary

The canonical implementation is the Rust workspace in `pheme-va/`, not the former Python `voice-agent/` service. Pheme VA currently owns portable audio processing, transcription adapter boundaries, the deterministic incident extractor, the development HTTP host, the local TUI, metrics, and the direct Whisper C ABI path.

The target Pheme workflow will own conversation/session state, clarification, corrections, confirmation, incident storage/retrieval, response generation, and validated action proposals. Those stateful capabilities are not implemented yet. The current Rust host is stateless and exposes `/v1/transcribe` and `/v1/analyze`; the current TUI has local developer run history, but that is not the planned incident/session database.

Go exposes the public API and owns benchmark orchestration and experiment results. At present it is wired to `MockIncidentAnalyzer` and `MockRunner`, with in-memory repositories. A future Pheme-backed Go analyzer/client should preserve the existing public endpoint and delegate stateful operations rather than duplicate workflow rules. The web app owns the future interaction/playback client and research dashboard. Each service accesses only its own repositories.

The [Pheme VA API contract](api/voice-agent.md) documents the current Rust host and the proposed stateful routes. It is not documentation for an active Python runtime.

## Functional requirements

The proposed first complete milestone is: report an incident by voice, answer a clarification, correct the draft, confirm it, and request a spoken summary of recent reports.

| Capability          | Expected behaviour                                                                                |
| ------------------- | ------------------------------------------------------------------------------------------------- |
| Speech input        | Transcribe a spoken report or query; recover from silence or unusable audio                       |
| Incident extraction | Extract supported facts and identify missing information without inventing details                |
| Conversation        | Ask relevant clarification questions and apply user corrections                                   |
| Human confirmation  | Read back the draft and require explicit confirmation of its current revision before finalization |
| Persistence         | Retain confirmed reports across restarts; prevent duplicate reports on retries                    |
| Retrieval           | Select records using validated filters and summarize only those records                           |
| Speech output       | Speak clarification questions, readbacks, and retrieval answers                                   |
| Voice console       | Provide recording/playback and development visibility into transcripts and drafts                 |
| Benchmark dashboard | Create experiments, track status, view results, compare configurations, and export                |

The initial interaction is push-to-talk, one turn at a time. Continuous listening, wake words, and interruption during playback are later refinements, not first-milestone requirements.

The current incident report shape has `incident_type`, `location`, `severity`, `summary`, and `recommended_action`. The Rust rule-based extractor and Go mock both return those fields, but they are different implementations: Rust copies simple explicit facts and uses `unknown` when it cannot extract a field; Go currently returns a placeholder recommendation. The initial analysis contract defines the response shape and unknown handling. Stakeholder report-completeness rules for finalization remain to be confirmed before the session workflow is implemented. Model-generated recommendations must never trigger real-world actions automatically.

For “give me the last five incident reports in the last hour,” the proposed default is recording time, newest first. Store occurrence time separately when known. Define time-zone and boundary semantics in the API contract. Return fewer than five when fewer exist, and explicitly handle no matches.

## Current implementation snapshot

| Area               | Current state                                                                                                                                           |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go public API      | Health, mock stateless incident analysis, asynchronous mock experiments, and in-memory metrics append/read                                              |
| Pheme VA core      | WAV validation, supported-format decoding, mono/16 kHz normalization, speech gate, transcript guards, cleanup seams, and rule-based incident extraction |
| Speech adapters    | Optional in-process whisper.cpp adapter and optional LiteRT Zipformer adapter; manifest-driven selection is in the CLI                                  |
| Pheme VA hosts     | Axum development server, Ratatui TUI, and direct-path Whisper C ABI                                                                                     |
| Metrics            | Rust typed pub/sub/batches and desktop resource sampling; Go validates/stores batches separately in memory                                              |
| Stateful reporting | Sessions, clarification, revision-bound confirmation, SQLite incident storage, retrieval, speech synthesis, and Go integration are not implemented      |
| Web                | Toolchain scaffold and placeholder page only                                                                                                            |

The TUI's JSON run history is a bounded local developer-console feature. It stores run metadata, transcripts, and metric events, but it is not a substitute for Pheme-owned incident/session persistence and does not make the operational workflow available.

## Research requirements

Measure transcription and report accuracy, conversation completion, retrieval correctness, response latency, memory, compute utilization, power, energy per task, and sustained thermal behaviour. Evaluate local accents, terminology, and relevant noise conditions with held-out examples.

Compare quality and efficiency under recorded workloads and configurations. Do not use CPU utilization as a power estimate or claim on-device inference when the device accesses a remote runtime. See the [benchmark methodology](benchmark-methodology.md).

## Pending decisions

| Decision                                        | Why it matters                                                      |
| ----------------------------------------------- | ------------------------------------------------------------------- |
| Device model, OS, RAM, accelerator, storage     | Determines feasible model sizes and integration                     |
| Battery, measurement access, thermal conditions | Determines power and endurance evaluation                           |
| Fully offline/on-device requirements            | Determines permitted execution and data paths                       |
| Languages, accents, terminology, report schema  | Determines dataset coverage and acceptance criteria                 |
| LLM, Whisper size/runtime, quantization, TTS    | Requires baseline evaluation; whisper.cpp is provisional            |
| Quality and response-time thresholds            | Needed before declaring deployment success                          |
| Operating cycle and endurance target            | “8–10 hours” was illustrative, not a confirmed acceptance threshold |
| Robotics integration                            | Possible context; robot control is not current implementation scope |
| Recording retention and dataset permissions     | Needed before collecting and retaining operational data             |

MERaLiON was suggested for Singaporean speech relevance. Its [official site](https://www.meralion.ai/) is a reference for later evaluation, not evidence that it fits the selected device.
