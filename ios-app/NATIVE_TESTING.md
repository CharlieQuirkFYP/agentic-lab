# On-device Pheme: build and test

Status: Expo owns the chat/settings UI and all Swift host code. Local transcription,
reasoning, bundled model preparation, transcript recovery and automated tests are
implemented, together with foreground SQLite metrics queuing and Go ingestion.
Local Release simulator linking, bundle validation, launch and
dual-model readiness passed. Foreground uploads from user-operated ASR/reply
turns on the paired iPhone were verified on 2026-11-09. On 2026-11-11, signed
Release builds executed the four-configuration matrix and recovered from a
forced app termination during a real Go outage, with exact phone/server record
reconciliation. The matrix retained nine configuration failures out of 24 jobs.
Network-disabled launch, human-speech quality, active-load sensor calibration
and energy/endurance acceptance remain open. See the
[physical acceptance record](../docs/testing/phone-benchmark-acceptance-2026-11-11.md).
Old `PhemePhone.app`/IPA artifacts are from the removed SwiftUI
host, not evidence for this Expo build.

The Results graphs have a separate [implementation validation record](../docs/testing/phone-results-acceptance-2026-11-13.md).
Earlier collector acceptance does not establish native graph responsiveness,
VoiceOver behavior, or rendering overhead on the physical phone.

The target is the user's iPhone 18 Pro; record its actual iOS version before selecting Xcode/SDK and signing settings. Expo Go cannot contain the custom Swift/Rust module. Use an installed development build, then a release-like standalone build for offline acceptance.

## Architecture and scope

```text
Expo chat and model settings
  -> PhemeNativeModule.swift -> PhemeRuntime.swift
  -> NativeBridge.swift -> PhemeVA -> Rust core / CPU Whisper
  -> ReasoningBridge.swift -> PhemeReasoning -> Rust / CPU llama.cpp
  -> bundled local models; saved local transcript/draft
```

There is no desktop inference fallback in either route. The previous HTTP client
is not imported by the app. Reasoning retains bounded successful history in Rust,
and resetting a chat clears that history without reloading weights. This is not
the full incident confirmation/storage/retrieval workflow. The root collector
persists native metrics and host model/timing metadata to an upload outbox.
Inference remains on-device regardless of whether metrics sync is enabled.
Native whole-app CPU, resident memory, physical footprint, battery/charging and
thermal state sampling are implemented. A short physical-device Instruments
comparison checked low-load CPU/memory agreement. GPU and verified power/energy
measurements remain unavailable.

Reasoning uses the core's `ConversationConfig::default()`: a 4,096-token context,
up to 128 output tokens, six retained successful pairs, 12,000 input characters
and 800 output characters. Sampling follows Qwen3-4B-Instruct-2507's official
baseline: temperature `0.7`, top-k `20`, top-p `0.8`, min-p `0`, with an optional
seed (default requests a native random seed). The former defaults allowed 512
tokens/4,000 characters and used `0.2`/`40`/`0.95`. These are shared development
defaults, not validated phone memory, quality or latency limits.

The bundled role's first line, `Pheme incident intake v1`, opts into Rust-owned
deterministic intake. Pheme conservatively copies approved caller evidence,
selects the next field question, and binds wording review to the current draft
revision; corrections invalidate that review. Candidate output is buffered and
allowlist-validated before display or speech. Delivered replies contain exactly
two sentences and at most 45 words; rejected, empty or overlong candidates use a
deterministic fallback. Backend errors/cancellation stay failed/cancelled, and
reset clears intake state and history without unloading weights. Each
conversation owns separate state; custom roles without the marker stay generic,
as do isolated Tests and finish-time title generation. No report is saved and no
help is dispatched. See [intake behavior and quality checks](../docs/incident-intake-quality.md).

Native calls have one process-wide owner scoped to each JavaScript context. A stale context cannot close its replacement. Both models share admission and an off-UI inference queue; status/metrics remain inspectable. ASR cancellation discards results; reasoning cancellation also requests cooperative native abort. A cancellation racing successful reasoning resets unseen history. Neither model is destroyed until admitted work settles. Backgrounding cancels delivery; memory warnings and JS teardown close both engines after settlement. Switching between chat/settings retains warm engines.

Expo prebuild applies `scripts/with-ios-scenes.cjs`: the generated app delegate
provides its React Native factory to Expo's own `ExpoAppSceneDelegate`, which
creates the window and forwards lifecycle/link events. iOS 27 rejects the old
application-only startup. The plugin is idempotent, rejects unsupported
templates, and is regression-tested; it does not add another native UI.

## Tests without Xcode, model weights or a device

From `ios-app/` with Node 22.18+:

```sh
npm ci
npm run typecheck
npm test
npm run test:native
```

`test:native` compiles the actual Foundation runtime, bridge types, bundle resolver
and AVFoundation WAV decoder with injected fake engines. It covers import/hash
failures, settings, warm reuse, WAV limits, shared admission, cancellation,
delayed destruction, JS-context replacement, bundle paths without copies, and
unseen reply-history reset. JavaScript tests cover chat-to-native wiring, reviewed
audio, direct typed replies, no remote fallback, recording persistence ordering,
SQLite migration, metrics retry/storage bounds, privacy, ingestion attribution
and production framework/resource checks.

The optional real HTTP boundary test requires a fresh disposable Go database
and a loopback API on a port other than 8080. It verifies the actual app outbox,
idempotency, dashboard notification, summary and JSON export, using explicitly
labelled fixture data:

```sh
# In api/, with an unused temporary database path:
API_ADDR=127.0.0.1:18080 API_DATABASE_PATH=/tmp/pheme-metrics-test.sqlite go run ./cmd/server
# In ios-app/, while that API is running:
PHEME_METRICS_TEST_URL=http://127.0.0.1:18080 node --test tests/metrics-http.test.ts
```

Stop that test API afterward. Never run the fixture test against the live phone
database; the test requires an empty store and does not measure a physical phone.

To additionally test the actual Rust-to-Swift C boundary without Whisper weights/native Whisper compilation:

```sh
cd ../pheme-va
CARGO_NET_OFFLINE=true bash scripts/test-ffi-swift.sh disabled test
```

Fetch locked Cargo dependencies first if the cache is empty. Arguments select
variants; the default tests disabled, Whisper and fake builds. The Swift sources
and smoke test now come from `ios-app/`. Fake entry points remain isolated and
must never ship. There is no standalone XcodeGen app or project-path test.

## Prepare the real native app

1. Install/select full Xcode 26.4 or newer and its simulator SDK. Expo SDK 57 requires Xcode 26.4 / Swift 6.3 or newer; the older Xcode 26.0 setting in the FFI-only matrix is insufficient for the Expo app. Set up device trust, Developer Mode and signing for the actual iPhone. Use an SDK that supports the phone's installed iOS. Pheme's build script pins Rust 1.99.0 and targets iOS 17+; the target phone's compatibility is an acceptance check. The consolidated iOS workflow's native-link job uses macOS 26 with Xcode 26.4 and builds ARM64, matching the packaged Rust simulator slice.
2. Build both native libraries and bundle the weights:

   ```sh
   cd ios-app
   npm ci
   PHEME_IOS_RUST_TOOLCHAIN=stable npm run native:build
   npm run native:bundle
   npm run native:check
   npx expo prebuild --platform ios
   npm run ios
   ```

3. Prebuild regenerates Expo's app delegate, scene configuration and pods, including when `ios/` predates this consolidation. Repeat after config/plugin changes; keep any required team/bundle settings in Expo configuration rather than relying on edits to ignored generated files.
4. Select the device as needed. Set a team-owned bundle identifier and
   `ios.appleTeamId` in `app.json` if required; the configured team retains this
   workspace's prior signing selection across prebuild. `npm run ios` prepares
   native inputs before calling `expo run:ios --device`. After installation,
   use `npx expo start --dev-client` for development.
5. Rebuild the native app after Swift/Rust changes. Swift sources are compiled
   directly from the Expo module, not copied from another host. Rebuild both
   XCFrameworks after Rust/header/Cargo changes with `npm run native:build`, then
   restore bundled mode with `npm run native:bundle` and check/prebuild the app
   before `npm run ios:release`. Install the rebuilt app on the phone; Metro
   refresh cannot update native binaries or a bundled startup role. Test with
   Metro and laptop inference stopped.

Canonical Swift source is `modules/pheme-native/ios/`. Preparation copies the
two production XCFrameworks into ignored `Frameworks/`. The optional
`Resources/PhemeModels/` becomes `PhemeModels.bundle` through CocoaPods. The app
reads bundled weights directly, without another phone-side copy. Preparation
checks Rust source/header/Cargo fingerprints, both library slices and production
symbols. Pod install fails on stale/missing inputs. Reasoning stays dynamic with
private GGML/llama symbols; never merge it into the static Whisper archive.

`native:prepare` preserves the last explicit resource selection. Use
`node scripts/prepare-native.mjs --without-models` for weight-free CI; use
`npm run native:bundle` to restore bundled models. PR CI downloads no weights.
The pinned Whisper large-v3-turbo/Qwen3-4B-Instruct-2507 Q4_K_M weights total
4,121,836,395 bytes. Qwen3 uses the checksum-pinned Unsloth GGUF and replaces
the bundled Qwen2.5-1.5B; the older model remains in the desktop catalog.
Device memory and latency suitability remains unvalidated.

Generated `ios/`, `android/`, Expo caches and native binaries are ignored. Commit module sources/configuration and lockfile, never model weights, XCFramework binaries or recordings. No EAS account/service is required for this local build path.

## First real-model test

On the laptop, optionally download the existing pinned smoke-test assets:

```sh
cd pheme-va
bash scripts/ios-model-fixture.sh
```

This downloads `ggml-tiny.en.bin` and `jfk.wav` into ignored `target/ios/assets/`. Transfer both to Files on the phone. These assets are integration fixtures, not selected production model/audio data. The tiny.en SHA-256 is:

```text
921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
```

In the Expo app:

1. For the bundled build, chat selects and loads bundled models sequentially.
   Record, review the transcript, then approve a local reply. Typed messages also
   exercise reasoning. No model setup on the phone is needed.
2. For a smaller ASR smoke test, open **On-device model settings** from chat.
3. Keep the tiny.en model name/checksum and choose **Import model file**. Import copies and hashes the file into app-owned storage.
4. Select the model and choose **Load model / apply settings**. Confirm ready state and actual artifact hash.
5. Choose **Transcribe a saved WAV**, select `jfk.wav`, and check text/run ID/timing. Return to chat for the reviewed reply.
6. Transcripts and artifact metadata persist in `local_transcriptions` in
   `voice-recordings.db`. Recordings remain until persistence succeeds. Saved
   chat display history is not restored as native model context.

The native limit is 1–120 seconds, bounded by the applied configuration. Model import is capped at 8 GiB; that is an input bound, not a claim that the device can load an 8 GiB model. WAVs are capped at 32 MiB, supported channel/rate/sample limits and the configured duration before allocating decoded PCM. No full PCM arrays/base64 traverse the JS bridge.

## Local verification record: 2026-11-03

- Xcode 27.2 (27B52), Rust stable 1.99.0; both production device and ARM64 simulator
  XCFrameworks rebuilt after switching reasoning to the core defaults.
- The complete Expo ARM64 simulator Release build passed. Finished-app validation
  passed for embedded JS, Expo scenes, native linkage/private-symbol isolation,
  both model sizes/SHA-256 hashes, notices and the incident role.
- Installed and launched `com.example.incidentreportingagent` in the iPhone 18 Pro
  iOS 27.0 simulator. The existing Expo chat rendered with both model filenames
  and a ready indicator; native logs confirmed the 4,096-token reasoning context.
- TypeScript checks, 20 JavaScript tests, fake-backed Swift runtime/WAV tests,
  relocated Swift C-ABI smoke tests, six reasoning C-ABI tests, Rust formatting,
  workspace Clippy and offline iOS packaging tests passed.
- `cargo test --workspace` remains non-green: the existing desktop CLI test
  `tui::download::tests::cancellation_terminates_script_group_and_allows_cleanup_for_retry`
  timed out waiting for cleanup. Tests for the remaining workspace packages passed;
  the CLI implementation was not changed by this migration.

No physical-phone installation or model-backed microphone/reply turn was verified
in this run. Simulator touch-driven tests were not completed: desktop accessibility
automation was unavailable, and route-opening requested system confirmation.
Launch/readiness is not end-to-end inference, quality or performance acceptance.

## Local verification record: 2026-11-09

- The metrics-enabled physical-device Release target compiled and signed with
  Xcode 27.2, version `0.1.0`, build `2`. Finished-app validation passed for
  embedded JS, Expo scenes, microphone/local-network permissions, restricted
  ATS configuration, portable native linkage, both model hashes and the role.
- The new Expo Swift `deviceInfo` API compiled in the full app, not only the
  fake-backed runtime tests. The bundled JS initially uses
  `http://192.168.1.40:8080`; saved metrics settings can change that address.
- TypeScript checks and 34 JavaScript tests passed; the optional HTTP test is
  skipped by the normal suite. Fake-backed Swift tests and native packaging
  validation passed. All Go tests/vet and phone-ingestion/handler race tests
  passed. Web tests and production build passed; lint reported eight warnings
  and no errors.
- A separate disposable Go API accepted the actual app outbox fixture and
  verified idempotent retry, dashboard notification, summaries and JSON export.
  That fixture is explicitly labelled as not a phone and is not in the live
  collector database.
- The LAN API exposes the new phone routes, and the dashboard's empty state was
  checked in a browser before installation, with zero phone batches.
- The signed app was installed and launched over the paired Wi-Fi connection
  on `iPhone19,2`, iOS `27.0.1`. The user completed a local reply. Go acknowledged
  uploads from the phone's LAN address and retained seven batches across five
  observation containers: two native ASR runs, their metadata, and one reply.
  Dashboard counts/charts updated automatically; the reply detail and JSON
  export retained phone attribution and null power/energy values.
- Reported ASR durations were about 4.23 and 4.28 seconds; the reported reply
  duration was 12.298 seconds. These individual observations are not latency
  acceptance criteria, quality evaluation or measured energy.
- A regression test ensures repeated native reply counters after engine/process
  restart have distinct upload identities by including the chat turn ID.
  The final signed update was installed and relaunched; the seven batches
  containing 56 events remained in Go. The counter fix is covered by the
  regression test; a new post-restart inference turn was not independently
  verified after that update. Full offline/restart/pause/permission and
  dropped-event acceptance still requires the checklist below.

## Local verification record: 2026-11-10

- Reasoning now bundles Qwen3-4B-Instruct-2507 Q4_K_M from Unsloth instead of
  Qwen2.5-1.5B. The new manifest/download pin records 2,497,281,120 bytes and
  SHA-256 `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597`.
  Whisper and the incident role are unchanged; the old reply entry remains
  available for desktop comparisons.
- Xcode 27.2 built and signed the full device Release app, version `0.1.0`,
  build `3`, with embedded JavaScript. Finished-app validation passed for both
  weight hashes, the exact catalog selections, absence of extra reply weights,
  role, Expo scenes, permissions, native linkage and deep code signing.
- TypeScript checks, 39 JavaScript tests, offline downloader regressions and
  fake-backed Swift lifecycle tests passed; the optional HTTP test was skipped.
  Rust formatting/linting, core tests and fresh-build non-CLI workspace tests
  passed. The full fresh-build suite still hit the previously recorded desktop
  TUI cancellation-test timeout (172 CLI tests passed, two were ignored).
- The real Qwen3 native worker completed three CPU-only incident turns with a
  4,096-token context, including a location correction and uncertain read-back,
  without thinking blocks. The opt-in real-model streaming/cancellation/fresh
  context test also passed. These Mac tests are not phone latency measurements.
- The first paired Wi-Fi installation timed out after 600 seconds; the phone
  still reported build `2`. The diagnostic retry was cancelled when USB became
  available. The same validated bundle installed over USB in about 128 seconds;
  the physical `iPhone19,2`, iOS `27.0.1`, reports version `0.1.0`, build `3`.
  The renewed development profile authorizes this phone and expires
  **2026-11-17**. After the user trusted/verified the developer profile, launch
  succeeded. The chat screen displayed the new Qwen3 filename and unchanged
  Whisper with a green readiness indicator; a user reply request was observed
  in progress without interrupting it. Completed phone reply, dual-model
  memory/latency and offline voice acceptance remain separate pending gates.

## Physical-device acceptance checklist

The intake/sampler update requires a new native build and phone installation.
Earlier verification records above retain their original build context;
they do not verify the updated intake. Run the ignored `intake_quality` and
`dictionary_quality` checks using the [documented environment variables](../docs/incident-intake-quality.md#reproduce-the-model-backed-checks),
record raw model acceptance separately from delivered validated replies, and
label synthetic/JFK fixtures separately from human Singapore audio.

- In the installed bundled build, complete successive incident, location,
  injury/uncertainty and ongoing-hazard turns. Check exactly two sentences and
  at most 45 words in each delivered reply, and verify rejected text is absent
  from display and speech.
- Correct an unfamiliar location explicitly, review the delivered excerpt,
  confirm its wording, then correct another field. Verify the earlier wording
  review is invalidated and the changed wording needs another review.
- Check save/dispatch requests describe the current capability limits and do
  not claim a saved report or dispatched help. Cancel/reset during a reply and
  start a new conversation; verify the cancelled turn and previous intake
  evidence do not leak into the new conversation.
- Compare dictionary hints on a consented human Singapore recording with a
  verified reference transcript, retaining uncertainty/negation checks. A
  passing synthetic fixture does not establish regional ASR quality.

Record date, iPhone model, iOS, app/build revision, library metadata, model SHA-256, thread count, maximum audio duration and execution location for each acceptance run.

- Stop laptop Go and Pheme. Transcribe the fixture and a microphone recording successfully.
- Repeat with a standalone/release-like build with embedded JS and Wi-Fi/mobile data disabled after model setup. Metro/Dev Client connectivity is not an inference requirement, but a dev server-dependent launch alone cannot prove offline acceptance.
- Repeat three turns with the same loaded model; inspect run IDs and verify model loading is not performed each turn.
- Reject a wrong model checksum, recover a missing/corrupt model, change settings and load another supported model.
- Test denied microphone permission, silence, stop/auto-stop at the configured limit, audio interruption, backgrounding and route changes.
- During a long operation, discard the result and try another operation. The UI must wait for native settlement and must not publish the discarded transcript.
- Reload JavaScript during a long operation, navigate back/reopen and confirm no second engine loads before settlement. Old cleanup must not close the replacement engine.
- Close/relaunch the app: the edited transcript and imported model/setup remain; load the model again before new inference. Pending local recordings can be retried explicitly.
- Exercise storage/persistence failure and verify the recovery recording is retained. Local inference retry starts a new run; no automatic duplicate execution is attempted.
- Force memory pressure where practical; confirm the engine closes after active work settles, then reopen the local screen and load the model again.
- Verify live metrics show actual run IDs and native/host drop counts. Do not
  report device power/energy as zero or assume desktop metrics describe the phone.

## Phone metrics acceptance

Follow [Wi-Fi setup](README.md#connect-metrics-over-wi-fi), then:

- Verify the installed build includes **Metrics sync** and the native
  `deviceInfo` method. Older native builds require rebuilding, not an export
  option or remote JavaScript update.
- Allow iOS Local Network access, complete a real ASR/reply turn, and check
  acknowledged batches plus matching hardware/OS/app/build attribution on the
  dashboard. Go stores `device_reported` observations without launching desktop
  inference. ASR, metadata and replies may have separate experiment containers.
- Stop Go, complete another turn, and confirm pending batches remain. Relaunch
  the app, restart Go, and verify the queue clears without duplicate batches.
- Pause uploads, produce observations, then resume. Local collection should
  continue while paused. Collection/upload resume only while foregrounded;
  background delivery is not implemented.
- Deny/re-enable Local Network permission in iOS Settings and confirm queued
  data is retained. Validate private-IP HTTP access in the actual Release build,
  not only on the Mac or simulator.
- Verify summaries keep differing source/scope/unit groups separate, preserve
  valid zeroes and show null power/energy as unavailable. JSON export retains
  device attribution; CSV currently has no dedicated device-attribution columns.
- Inspect the wire payload: no recording, transcript, vocabulary hint or reply
  text should be uploaded by the app's producers. Acknowledgment means storage,
  not independently verified measurements or a finalized incident report.

The queue is bounded to 2,000 batches / 8 MiB. It does not silently evict unsent
data. A drained observation retained after a storage failure is only in memory
until SQLite succeeds; a crash can lose it, and native overflow remains possible.
Permanent rejected uploads remain blocked for inspection; changing the server
does not automatically unblock or rewrite them. See the
[metrics API](../docs/api/metrics.md) for the full contract.

## CI and remaining gates

[CI / iOS](../.github/workflows/ci-ios.yml) owns all iOS checks:

- `contracts` tests TypeScript, storage, native chat and fake-backed Swift
  behavior, then bundles Expo JS.
- `native-link` builds both production device/simulator frameworks once, checks
  the real Whisper Swift C bridge on macOS, uploads both frameworks, installs pods
  and compiles the complete ARM64 Expo simulator app without signing.
- `ios` retains disabled/test device/simulator packages and their Swift C-ABI
  smoke tests. Production Whisper is not rebuilt in this matrix.

The offline packaging orchestration suite runs once in
[CI / Pheme](../.github/workflows/ci-pheme.yml). CI downloads no weights and
retains native build diagnostics as artifacts. See the
[workflow inventory](../.github/README.md) for triggers and check-name migration.

The first successful native-link CI job establishes compilation/linking only. It does not replace the real-model/device checks above or establish model quality, memory endurance, power, thermal behavior, native microphone correctness or full local conversations.

Follow-on work: structured incident storage workflow, verified energy/GPU
providers, and completion of measured physical-device acceptance. Native
CPU/memory/battery/thermal sampling and phone sweep/report tooling are now
implemented; their existence does not establish sensor accuracy or model quality.
The [deployment plan](../docs/plans/expo-on-device-pheme-plan.md) retains the
historical milestone breakdown; current ownership and commands are here.

## Conversation benchmark acceptance

The phone Benchmark screen records/imports ordered WAV turns, pins a dataset
revision, runs a model matrix sequentially and exports local JSON/CSV/Markdown.
It uses the same native runtimes and durable observation collector as manual
chat. Go receives observations; it does not execute the phone's inference.
See [the approved plan](../docs/plans/phone-conversation-benchmarks-plan.md),
[wire contract](../docs/plans/phone-benchmark-implementation-contract.md) and
[isolated HTTP fixture](../docs/testing/phone-observation-http.md).

Prepare the explicit four-model acceptance bundle from `ios-app/` after building
both production XCFrameworks:

```sh
node scripts/prepare-native.mjs --bundle-models \
  --asr-models whisper-large-v3-turbo,whisper-tiny.en \
  --reasoning-models qwen3-4b-instruct-2507-q4-k-m,qwen2.5-1.5b-instruct-q4-k-m
node scripts/device-benchmark.mjs --help
```

The physical-only script stages WAVs, launches the installed Release app and
copies its durable evidence from `Documents/BenchmarkAcceptance/<id>/`.
`--sync live` permits uploads during inference; `paused` keeps observations local.
`--sample-resources 0|1` records an explicit sampler condition. Preserve failed
executions and use a new ID for a rerun; `--action resume` retains interrupted
executions and restarts the unfinished conversation from turn one.

Model catalogue/settings operations share native admission and must run
sequentially. Rust serializes accepted ASR status as `Speech`; metric status
labels use `speech`. A configuration block releases the prior native pair before
loading its next pair; both models stay loaded between turns in that block.
Actual device reports must remain separate from fake/native ABI/HTTP fixtures.

The 2026-11-11 physical acceptance record is in
[`docs/testing/phone-benchmark-acceptance-2026-11-11.md`](../docs/testing/phone-benchmark-acceptance-2026-11-11.md).
It records the 24-job matrix (15 successful, nine configuration failures), exact
phone/Go reconciliation, forced app restart during a real Go outage, the
three-conversation sampler-off/on controls and a short Instruments CPU/memory
comparison. The current installed Release build is `0.1.0 (8)`. The main matrix
retains build-7 attribution; regenerating its report in build 8 did not relabel
or rerun those measurements. Network-disabled launch, controlled active-load
calibration, human-speech quality and measured energy/endurance remain open.
