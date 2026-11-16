# Incident intake behavior and quality evaluation

Pheme's bundled incident role collects a bounded, in-memory account from approved
caller text. Pheme selects the next intake question and validates the delivered
reply; the language model supplies a candidate response. This is development
intake, with wording review and corrections, before the planned persistent
incident-report workflow.

## Role selection and response contract

The first line of the combined startup role must be exactly:

```text
Pheme incident intake v1
```

The bundled [`incident-reporting.txt`](../pheme-va/roles/incident-reporting.txt)
opts into this behavior. Existing trusted custom roles remain selectable. A
role without this first-line marker uses generic conversation behavior. Combining
roles still preserves their selected order and exact-byte prompt hash; placing
the marker in a later file or inside caller text does not activate intake.

Each delivered intake reply contains exactly two sentences and at most 45 words.
Pheme builds a response plan from approved text and the conversation's successful
intake state. The model receives that selected response as an instruction; it
cannot choose a different question, add tactical advice, or authorize an action.
Validation accepts the planned response or a small allowlist of acknowledgements
paired with the same selected second sentence.

Candidate model chunks are buffered until validation completes. A rejected,
empty or overlong candidate becomes the deterministic planned response without
a second inference. Rejected candidate text never becomes a chat delta, a
completed reply or speech input. Backend failures and cancellation remain failed
or cancelled operations; they do not become successful fallback replies.
Generic custom-role conversations and isolated Tests retain their raw model
streaming behavior. Finish-time title generation uses its own generic title
instruction and does not run the incident intake state machine.

## Evidence, questions and wording review

The deterministic intake copies bounded evidence from approved caller wording
into incident, location, injury and ongoing-hazard fields. It uses conservative
clause and keyword rules, including short answers to the field currently being
requested. This is not model-backed structured extraction or a place-name
resolver. Caller wording such as `Amokil` is not silently changed to `Ang Mo Kio`;
an explicit caller correction can replace the location. Uncertainty and negated
injury statements remain caller evidence rather than inferred facts.

The normal question order is fixed:

| Missing or unresolved detail | Selected question |
| --- | --- |
| Incident | What happened? |
| Location | What nearby road or landmark identifies the location? |
| Conflicting locations | Which location is correct? |
| Injury information | Is anyone injured or trapped? |
| Ongoing hazards | Are there any ongoing hazards? |

After these fields are available, the workflow can present a bounded excerpt
for wording review. Capability and immediate-danger responses can defer the
excerpt rather than treating an undisplayed draft as reviewed. The wording
question applies only after Pheme has actually delivered the excerpt. A positive
answer records the current draft revision as reviewed; a correction invalidates
that review, updates the evidence or requests clarification, and requires review
of the changed wording. The excerpt can omit parts of a long account and must
not be described as a complete saved report.

State belongs to each conversation. It is separate from the six-pair language
model history window, so retained fields do not depend on every earlier message
remaining in the prompt. A failed or cancelled reply commits neither its planned
state change nor a successful history pair. Reset clears intake fields and
history while retaining loaded model weights; a new conversation starts with
fresh intake state. Isolated Tests do not alter the web or console conversation.

Requests to save, finalize, retrieve or dispatch receive truthful capability
wording. This interface performs none of those operations. A successful wording
check is not authorization to save a report. Persistent Pheme-owned sessions,
revision-bound report finalization, database queries and retrieval remain planned.

## Shared generation configuration

`ConversationConfig::default()` is shared by desktop and native reasoning:

| Setting | Current default | Previous default where changed |
| --- | --- | --- |
| Context tokens | 4,096 | Unchanged |
| Maximum output tokens | 128 | 512 |
| Maximum output Unicode scalar values | 800 | 4,000 |
| Successful history pairs | 6 | Unchanged |
| Aggregate input Unicode scalar values | 12,000 | Unchanged |
| Temperature | 0.7 | 0.2 |
| Top-k | 20 | 40 |
| Top-p | 0.8 | 0.95 |
| Min-p | 0 | Unchanged |
| Seed | Optional; default requests a native random seed | Evaluation can pin a seed |

The input budget includes system instructions, selected history and approved
text; actual token overflow is still checked by the adapter. The output budget
bounds generation in addition to the intake's two-sentence/45-word contract.
The inspector's `conversation_config` reports the actual configured settings.

The sampling values follow the official
[Qwen3-4B-Instruct-2507 model card](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507#best-practices).
That card describes a non-thinking model and recommends these sampling values.
Pheme implements the sampler through
[llama.cpp](https://github.com/ggml-org/llama.cpp/blob/master/src/llama-sampling.cpp).
The short output budget is an application choice for this voice interaction,
not the model card's general-purpose output-length recommendation. These choices
do not establish improved incident understanding, phone latency or memory
acceptance. Other selected models may require their own measured configuration.

## Reproduce the model-backed checks

Normal tests use fakes and small fixtures. These ignored tests load explicitly
supplied local weights; they do not download or write weights, recordings or
result artifacts. Run them from `pheme-va/`, and keep any captured output outside
Git. Native compilation needs CMake, a C/C++ compiler and libclang.

### Raw reply and delivered intake comparison

```sh
PHEME_VA_REPLY_MODEL=/absolute/path/to/qwen.gguf \
PHEME_VA_REPLY_PROMPT=/absolute/path/to/pheme-va/roles/incident-reporting.txt \
cargo test -p reply-native --test intake_quality -- --ignored --nocapture --test-threads=1
```

The test loads one CPU model with four threads and no GPU layers. It compares
legacy sampling (`0.2`/`40`/`0.95`/`0`) with current Qwen sampling
(`0.7`/`20`/`0.8`/`0`), both with seed `42` and the same 128-token/800-character
budgets. It runs a nine-turn incident conversation for each configuration,
including unfamiliar location wording, uncertainty, corrections, wording review,
unsupported save/dispatch requests and attempts to override the role.

Output separates candidate model text, raw allowlist acceptance, adapter errors,
the validated delivered reply, fallback use, word counts and elapsed time. The
test asserts the final response contract, one inference per turn, successful
history/state commit and suppression of rejected chunks. It does not assert that
current sampling always improves raw model acceptance. A fully compliant final
reply can be a deterministic fallback and therefore is not evidence that the
model itself followed the instruction. The fixed legacy-then-current order and
small sequential fixture are a regression comparison, not a statistical quality
or phone-performance benchmark.

### Plain versus dictionary-hinted transcription

```sh
PHEME_VA_WHISPER_MODEL=/absolute/path/to/whisper.bin \
PHEME_VA_TEST_WAV=/absolute/path/to/recording.wav \
PHEME_VA_TEST_REFERENCE='verbatim expected transcript text' \
PHEME_VA_TEST_DICTIONARY='Amokil,Ang Mo Kio,oil tankers' \
cargo test -p whispercpp --test dictionary_quality -- --ignored --nocapture --test-threads=1
```

`PHEME_VA_TEST_REFERENCE` contains literal reference text, not a file path.
`PHEME_VA_TEST_DICTIONARY` is comma-separated vocabulary. Optionally set
`PHEME_VA_TEST_LANGUAGE=en`; otherwise language is auto-detected. The same loaded
engine transcribes the same WAV first without and then with the dictionary hint.
Cleanup is disabled; the default prompt-echo retry guard remains enabled.

Output includes normalized word edit distance/WER, expected versus observed
dictionary-term correctness, raw/final transcripts, prompt text, recovery status,
elapsed/processing time and the difference between runs. Hints are recognizer
context, not forced replacements. A hint can improve, worsen or leave a phrase
unchanged, and the test does not universally assert improvement. The phone's
dictionary field accepts natural comma-separated entries, trims surrounding
whitespace and ignores blank entries; the native configuration carries the
resulting terms through to the recognizer.

Synthetic speech and the downloaded JFK smoke-test WAV are integration fixtures.
They do not represent human Singapore speech, local accents, noisy incident
conditions or microphone quality. Record and label human Singapore audio
separately, with a verified literal reference transcript and appropriate consent,
before making regional ASR-quality claims.

## Results and native acceptance

Local checks on 2026-11-10 used a Mac15,7 (arm64, 36 GiB RAM), four CPU
threads, and no model GPU offload. The source was the modified working tree
based on `632bde8`; the role SHA-256 was
`8f3d4dca7cfe9212906f88f5d49c9d9d28feeaf5a0b2d854d9e455f0dbdeae4f`,
and the intake implementation SHA-256 was
`a08f4562e97f944ce38bd255cb8599ccfdf7a93a85887ca6d77ab6a1b0ecf292`.
The native comparison uses equal output/context budgets and seed 42; timings
are single-run observations, not a controlled latency benchmark.

| Sampling configuration | Raw responses accepted by the intake contract | Delivered responses accepted |
| --- | --- | --- |
| Legacy: temperature 0.2, top-k 40, top-p 0.95 | 4 / 9 | 9 / 9 |
| Qwen recommendation: temperature 0.7, top-k 20, top-p 0.8, min-p 0 | 4 / 9 | 9 / 9 |

All eighteen native model calls completed without adapter errors. Five turns
per configuration used the deterministic fallback. The complete tanker excerpt
retained the final clause “The fire is still burning”; explicit requests to
dispatch or save received the planned capability limits. The raw model still
repeated earlier questions and produced extra wording in some turns.

This measures adherence to a deliberately narrow, allowlisted response contract,
not overall incident understanding or intelligence. The same prompt and intake
state machine were used for both sampling configurations, so this is a
sampling-only comparison. One fixed seed, nine turns and legacy-first order do
not support a general quality or latency ranking. Sampling alone showed no
adherence gain here; the deterministic state and validator supplied the reliable
delivered behavior. Observed generation totals were 69,086 ms for legacy and
65,736 ms for Qwen, excluding model load.

The ASR fixture was generated locally with macOS `say`, Samantha voice at
145 words/minute, then converted to mono 16 kHz PCM16 WAV. Its 4.605375-second
audio SHA-256 was
`017c33e0769dbe489164dfa3dcca313b2946677f9a9c7c47cd574391ed031f37`.
The literal reference was “There is smoke at Jalan Buroh near the west entrance.
Nobody is injured.” The configured language was `en`, with dictionary entries
`Jalan Buroh` and `west entrance`.

| Whisper mode | Word errors / reference words | WER | Place-name result |
| --- | --- | --- | --- |
| Plain | 1 / 13 | 0.0769 | Jalan Bureau |
| Dictionary hinted | 0 / 13 | 0.0000 | Jalan Buroh |

Neither decode needed dictionary-echo recovery. The observed processing times
were 5,120 ms plain and 8,154 ms hinted; this fixture demonstrates a spelling
improvement and does not establish a speed improvement. Both are real local
Whisper decodes, but the speech is synthetic.

The local artifacts were:

| Artifact | SHA-256 |
| --- | --- |
| Qwen3-4B-Instruct-2507-Q4_K_M.gguf | `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597` |
| ggml-large-v3-turbo.bin | `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69` |

Mac CPU checks and synthetic fixtures do not establish physical-phone quality,
memory, thermal, latency, offline or speech-playback acceptance. Power/energy
remain unavailable. Human Singapore audio and physical-phone acceptance still
need separate, labelled evaluation.

Verification passed: Rust formatting, workspace Clippy with warnings denied,
345 workspace tests (eight opt-in tests ignored by default), both real-model
evaluations above, eight Go/Pheme integration checks, iOS TypeScript and Node
tests, fake Swift lifecycle tests, and Expo iOS JavaScript export. Both production
XCFrameworks were rebuilt for device and simulator, copied into the bundled Expo
module, and passed `native:check`. These packaging checks do not install the app
or replace physical-phone acceptance.

Rust changes require rebuilding **both** XCFrameworks before preparing the Expo
bundle. From `ios-app/`, use the existing bundled mode:

```sh
PHEME_IOS_RUST_TOOLCHAIN=stable npm run native:build
npm run native:bundle
npm run native:check
npx expo prebuild --platform ios
npm run ios:release
```

Install the rebuilt app on the phone; a Metro JavaScript refresh cannot update
Rust binaries or the bundled startup role. Check the delivered two-sentence
response, successive field questions, uncertainty, explicit corrections,
revision-bound wording review, cancellation/reset isolation and absence of
rejected text in display or speech. Recheck dual-model memory/latency and offline
operation on the installed build. See
[native testing and the physical-device checklist](../ios-app/NATIVE_TESTING.md).
