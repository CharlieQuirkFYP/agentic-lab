# Requirements and Scope

This document records the latest KLASS discussion and the resulting implementation plan. Planned capabilities below are not claims about the current code. See [architecture](architecture.md) for implementation status and [roadmap](roadmap.md) for delivery steps.

## Confirmed Direction

* The team builds the use-case solution itself, using pretrained models and libraries where appropriate. Existing KLASS solutions and adapters are excluded.
* Voice-Based Incident Reporting is the active use case, ideally speech-to-speech with agentic clarification and a human in the loop.
* Use Case 3, vision/licence-plate monitoring, is removed. Interview understanding is outside the current development focus but was not explicitly removed.
* Efficient local deployment is a core research objective. Power consumption must be measured alongside quality and performance.
* Whisper and llama.cpp are the working baseline discussed with KLASS, not a benchmark-validated device configuration.
* Local speech nuances should inform model/dataset evaluation and adaptation. MERaLiON is optional, not mandated.
* A mobile phone is the preferred eventual platform. KLASS may supply an iOS device or NVIDIA Jetson; selection and specifications are pending.
* Development should proceed without waiting for hardware.

## Service Boundary

The dedicated Python **Voice Agent** service will live in `voice-agent/`. It owns transcription, interpretation, the incident conversation/state machine, confirmation, incident storage/retrieval, response generation, and its Whisper/llama.cpp adapters. Go exposes the public API and owns benchmark orchestration and experiment results. The web app owns interaction/playback and the research dashboard. Each service accesses only its own repositories.

The initial text-analysis contract is specified in [Voice Agent API](api/voice-agent.md); session contracts there remain an outline for later implementation. The service directory is not scaffolded by this documentation ticket.

## Functional Requirements

The proposed first complete milestone is: report an incident by voice, answer a clarification, correct the draft, confirm it, and request a spoken summary of recent reports.

| Capability | Expected behaviour |
| --- | --- |
| Speech input | Transcribe a spoken report or query; recover from silence or unusable audio |
| Incident extraction | Extract supported facts and identify missing information without inventing details |
| Conversation | Ask relevant clarification questions and apply user corrections |
| Human confirmation | Read back the draft and require explicit confirmation of its current revision before finalization |
| Persistence | Retain confirmed reports across restarts; prevent duplicate reports on retries |
| Retrieval | Select records using validated filters and summarize only those records |
| Speech output | Speak clarification questions, readbacks, and retrieval answers |
| Voice console | Provide recording/playback and development visibility into transcripts and drafts |
| Benchmark dashboard | Create experiments, track status, view results, compare configurations, and export |

The initial interaction is push-to-talk, one turn at a time. Continuous listening, wake words, and interruption during playback are later refinements, not first-milestone requirements.

The current report has incident type, location, severity, summary, and recommended action. The initial analysis contract defines required response fields, baseline allowed values, and unknown handling. Stakeholder report-completeness rules for finalization remain to be confirmed before the session workflow is implemented. Model-generated recommendations must not trigger real-world actions automatically.

For “give me the last five incident reports in the last hour,” the proposed default is recording time, newest first. Store occurrence time separately when known. Define time-zone and boundary semantics in the API contract. Return fewer than five when fewer exist, and explicitly handle no matches.

## Research Requirements

Measure transcription and report accuracy, conversation completion, retrieval correctness, response latency, memory, compute utilization, power, energy per task, and sustained thermal behaviour. Evaluate local accents, terminology, and relevant noise conditions with held-out examples.

Compare quality and efficiency under recorded workloads and configurations. Do not use CPU utilization as a power estimate or claim on-device inference when the device accesses a remote runtime. See the [benchmark methodology](benchmark-methodology.md).

## Pending Decisions

| Decision | Why it matters |
| --- | --- |
| Device model, OS, RAM, accelerator, storage | Determines feasible model sizes and integration |
| Battery, measurement access, thermal conditions | Determines power and endurance evaluation |
| Fully offline/on-device requirements | Determines permitted execution and data paths |
| Languages, accents, terminology, report schema | Determines dataset coverage and acceptance criteria |
| LLM, Whisper size/runtime, quantization, TTS | Requires baseline evaluation; whisper.cpp is provisional |
| Quality and response-time thresholds | Needed before declaring deployment success |
| Operating cycle and endurance target | “8–10 hours” was illustrative, not a confirmed acceptance threshold |
| Robotics integration | Possible context; robot control is not current implementation scope |
| Recording retention and dataset permissions | Needed before collecting and retaining operational data |

MERaLiON was suggested for Singaporean speech relevance. Its [official site](https://www.meralion.ai/) is a reference for later evaluation, not evidence that it fits the selected device.
