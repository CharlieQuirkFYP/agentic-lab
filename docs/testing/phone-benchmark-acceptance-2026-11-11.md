# Physical iPhone benchmark acceptance — 2026-11-11

The phone archive, sequential sweep runner, native telemetry, Go observation API,
current-first dashboard and JSON/CSV/Markdown reports were exercised on the paired
physical iPhone. The main matrix completed 15 of 24 planned conversations; nine
jobs failed during configuration warmup. Failed jobs remain visible and are not
included as zero-duration successes. This is pipeline and reporting acceptance,
not a validated model recommendation or incident-quality benchmark.

Device: `iPhone19,2` (iPhone 18 Pro), iOS `27.0.1` (`24A446`), app `0.1.0`.
The main matrix records identify build `7`; installed build `8` regenerated its
normalized report without repeating inference and performed the recovery tests.
Release signing, installation, model execution and evidence extraction used the
physical CoreDevice target. Go on port 8080 received observations and performed
no inference for these jobs.

The input was three ordered conversations with two synthetic WAV utterances
each, repeated twice for four ASR/reasoning combinations. The phone read the
same checksum-verified audio in every configuration. Dataset revision:
`588ef5d7c286eec8ba4c513bd0e6b6914f318380b9a0d6196a63d2c5de028776`.
Four CPU threads were configured, USB was connected and the main matrix recorded
serious thermal state. Audio was generated on the Mac; metrics and inference
were produced on the phone. Human speech, incident correctness and spoken output
latency are outside this pilot's evidence.

| ASR | Reasoning | Completed jobs | ASR mean ms (successful attempts) | Reasoning mean ms (successful attempts) |
| --- | --- | ---: | ---: | ---: |
| Whisper tiny.en | Qwen2.5 1.5B Q4_K_M | 6 / 6 | 239.44 (12) | 9,347.04 (12) |
| Whisper large-v3-turbo | Qwen2.5 1.5B Q4_K_M | 3 / 6 | 7,542.59 (6) | 9,123.82 (6) |
| Whisper large-v3-turbo | Qwen3 4B Q4_K_M | 0 / 6 | Unavailable | Unavailable |
| Whisper tiny.en | Qwen3 4B Q4_K_M | 6 / 6 | 244.08 (12) | 26,181.95 (12) |

Loading and warmup are separate attempts. The failures were recorded as native
cancellations during warmup after model loading; an operating-system cause was
not captured. Memory pressure remains a hypothesis. Large-v3-turbo with Qwen2.5
completed its first block and failed its second, so the evidence does not prove
absolute hardware incompatibility. p95 is unavailable below 20 eligible complete
successful observations per compatible group. Conditions and sample counts do
not support a controlled model ranking.

The main sweep `2552f8d4-8f7b-4443-954d-b784fd20500e` contains 2,159 immutable
records, 22,758 metric events, 15 conversations, 30 turns and 89 stage attempts.
Every phone record and its device attribution matches Go exactly: no missing,
extra or conflicting payloads, no reported drops, sequence gaps or unresolved
references. JSON report version 2 stores raw records once and reconstructs the
expanded hierarchy; this fixed the large export's phone string-size failure.
Build 8 exported the full matrix on the phone successfully.

A separate six-job build-8 run started with Go genuinely stopped. The app process
was forcibly terminated during its second conversation and then resumed. Its
first completed job kept one execution. The interrupted second execution remains
`interrupted`, `partial`, with reason `process_restart` and unavailable duration;
the retried conversation has a new execution ID. All six jobs subsequently
succeeded. All 644 retained phone records, including the interruption, match the
restarted Go service exactly. This verifies a server outage and process recovery;
it does not verify launch with Wi-Fi/mobile data disabled.

Native metrics include whole-app process CPU on a one-core basis, resident
memory, physical footprint, battery/charging, brightness and thermal category.
A roughly five-second Apple Instruments trace on this same phone after inference
recorded mean CPU 9.91% versus native 10.46%, resident memory 1,984,798,720 versus
1,984,796,486 bytes, and physical footprint 756,256,000 versus 756,165,143 bytes.
Sampling cadences differ and Instruments adds work. This is a short low-load
scope/sanity check, not active-inference calibration. GPU, watts and joules remain
unavailable; battery percentage is not measured energy.

The sampler-off and sampler-on controls each completed three conversations on
build 8 with Tiny.en/Qwen2.5 and uploads paused. Sampling off retained stage
timings and produced no resource events. Sampling on produced 810 events across
CPU, resident memory, footprint, thermal state and battery level. Mean ASR times
were 201.05 ms off / 216.61 ms on; reasoning was 8,564.28 / 9,453.49 ms; active
conversation duration was 17,530.65 / 19,340.19 ms. These are descriptive results,
not a measured 10.32% sampler cost: the controls ran in fixed order, replies use
stochastic decoding and thermal equivalence was not established. The on run
recorded serious thermal state; off removes that sensor observation. A repeated,
counterbalanced protocol is needed to estimate overhead reliably. The control
disables resource sampling only, not archive/lifecycle collection or stage timing.
Both control exports reconcile exactly with Go: 135 records / 258 events off,
295 records / 2,669 events on, with no missing, extra or conflicting payloads.
All seven physical runs in the generated ledger have exact reconciliation.
The final live browser check confirmed the latest control's three conversations
and 2,669 events; phone uploads were left enabled and Go/dashboard running.

Automated validation is separate from physical evidence: Go formatting/vet/tests,
32 web tests, phone tests (80 passed, two skipped), TypeScript, Swift lifecycle
checks, shared reporting tests and the isolated real-HTTP fixture passed. The
fixture covers 550 records and never writes fixture measurements to live port
8080. Browser checks verified current-first ordering, the retained overview,
attributed conversation details, filter round trips, three export formats and
desktop/mobile layout without browser errors. Rust Clippy passed; the full Rust
test suite is **not green** because the existing download-process cancellation
test times out. A serial run excluding only that test passed 344 tests, with
eight ignored and one filtered.

Generated evidence is intentionally ignored under
`ios-app/.expo/phone-acceptance-20261110/`. The readable
`physical-acceptance-report.md` links each copied phone report, reconciliation,
run ledger and acceptance check. `physical-acceptance-index.json` locates the
runs; `outage-recovery-verification.json` records recovery assertions;
`phone-idle-reference-comparison.json` and `phone-idle-activity.trace` retain the
Instruments comparison. Raw WAVs, model weights and generated reports must not
be committed. The live dashboard is `http://localhost:5173/dashboard`, backed by
the existing Go database on port 8080.
