# Benchmark Methodology Plan

This is a proposed evaluation protocol. The current runner returns hardcoded values and implements none of the measurements below. Acceptance thresholds and device-specific instrumentation remain pending.

## Evaluation Questions

1. Can users accurately create, correct, confirm, and retrieve incident reports through speech?
2. How do model/runtime configurations affect quality, response time, memory, and energy?
3. Can the selected configuration sustain the agreed operating cycle on the target device?
4. Do local-speech adaptations improve held-out results enough to justify their resource cost?

## Scenarios and Dataset

Include complete/incomplete reports, local accents and terminology, place names/numbers, background noise, silence, corrections, ambiguous confirmation, cancellation, and retrieval with zero/fewer/more than five matches. Annotate reference transcripts, expected facts, relevant report IDs, and allowed workflow outcomes.

Use consented recordings. Keep versioned manifests and intentionally small fixtures in Git; store larger audio and generated artifacts externally. Separate development and held-out evaluation data, preferably by speaker, and do not tune against the held-out set. Record dataset revision and scoring rules.

## Measurements

| Dimension           | Measures and interpretation                                                                                       |
| ------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Transcription       | Word error rate plus accuracy of important places, names, and numbers                                             |
| Report quality      | Field accuracy/completeness, unsupported facts, summary faithfulness                                              |
| Conversation        | Completion rate, clarification turns, correction success, incorrect finalization                                  |
| Retrieval           | Correct records, time boundaries, ordering, grounded summaries                                                    |
| Timing              | ASR, LLM, retrieval, and TTS durations; end-of-user-speech to first audible reply; full report duration           |
| Resources           | Defined process/system memory scope and available CPU/GPU utilization                                             |
| Power               | Average/peak watts during defined idle and active intervals                                                       |
| Energy              | Integrated power over time, expressed as joules or watt-hours per task; include failed attempts in the accounting |
| Sustained operation | Temperature, throttling indicators where available, latency drift, battery endurance                              |

Record unavailable metrics as absent with a reason, never zero. CPU utilization is not measured power. On systems with shared memory, document accounting to avoid presenting a naive sum of process memory as unique physical usage.

The initial implementation emits the application and resource events described in the [metrics contract](api/metrics.md). Process and system CPU are intentionally separate, as are process and system RAM scopes. Linux/macOS/Windows use the initial `sysinfo` sampler for CPU/RAM; GPU, temperature, battery, whole-device power, and energy require a host provider unless an external instrument supplies them. Power-to-energy integration is valid only when whole-device power samples are available.

Distinguish successful execution from task quality: a completed run can produce an inaccurate report. Report both. Document denominators for energy per completed report and task success.

## Repeatability

Record the code revision, prompts/schema versions, ASR/LLM/TTS model identities and revisions, quantization, runtime builds, decoding/context settings, dataset version, device/OS, power mode, and measurement method. Report whether inference and speech output are local or remote.

Separate cold-start/model-loading runs from warm runs. Use repeated trials and report sample counts and variation, with latency percentiles where sample size supports them. Fix or record workload, concurrency, audio lengths, background processes, ambient conditions, and starting battery conditions. Use one benchmark run at a time for the initial constrained-device baseline to limit interference.

The Go runner should exercise the Voice Agent service API through the shared client, with scripted human turns for repeatability. Voice Agent owns workflow execution and incident data; Go owns experiment configurations/results and scoring. Keep evaluation sessions/reports isolated from interactive data, and record the reset/cleanup procedure for reproducible retrieval scenarios. Include audio and speech-output measurements when claiming end-to-end results; text-only runs must be labelled separately. Human evaluation is needed for qualities not captured by deterministic scoring.

## Device Power and Endurance

Select instrumentation after device specifications arrive. Record whether measurements cover the whole device, an accelerator, or selected processes. Whole-device battery assessment includes idle/listening, speech output, display/network activity where applicable, and other platform loads. Robot-wide energy must be distinguished from compute-only energy if robotics becomes part of the scope.

Define a duty cycle: reports and queries per hour, typical turn/audio lengths, idle/listening time, and display usage. Treat the discussed 8–10 hours as an illustrative target until agreed with KLASS. A short inference measurement or battery-capacity estimate is not a validated endurance test.

## Implementation and Presentation

Extend the existing benchmark result contract incrementally, retaining current fields where possible. Store per-stage traces and complete configuration metadata alongside aggregate results. Add persistent results and dashboard filtering/comparison/export. Mark incompatible datasets or measurement scopes rather than ranking unlike runs without explanation.

Start with a bounded baseline comparison. Evaluate model size, quantization, context/output limits, and runtime settings before expanding the search. Investigate local vocabulary/prompt changes, alternative speech models, MERaLiON, or fine-tuning only when error analysis motivates them. See [roadmap](roadmap.md) for dependencies.
