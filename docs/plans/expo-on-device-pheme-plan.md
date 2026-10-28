# Expo on-device Pheme integration plan

Status: implementation started, 28 October 2026. The native module, local transcription route, model import, recovery storage and automated tests are implemented. Full Expo native linking and physical-device acceptance remain pending. See [build/testing instructions](../../ios-app/NATIVE_TESTING.md).

## Deployment target

The user confirmed an iPhone 18 Pro as the target. Its exact iOS version, available storage and signing setup remain to be recorded. Do not infer memory capacity, usable model size or performance from the device name.

```text
iPhone
  Expo / React Native interface
    -> local Expo native module (Swift)
    -> embedded Pheme VA Rust library
    -> local model inference
    -> local inference records and upload outbox
               |
               | Acknowledged metric/result uploads
               v
Laptop
  Go API -> Go-owned SQLite -> benchmark dashboard
```

The intended deployment runs inference on the phone. The laptop stores and displays uploaded measurements. SSE/WebSocket are transport options, not the boundary between JavaScript and Rust; local calls use the native bridge. Prefer the existing HTTP metric-batch contract for acknowledged uploads. The existing dashboard WebSocket can continue delivering update notifications.

This target replaces the remote-inference deployment direction in [the original iOS app draft](ios-app-draft.md). That draft still explains the existing API-backed client. Moving full inference onto the phone requires more than replacing its network URL.

## First milestone

**An installed Expo development build transcribes a saved WAV using embedded Pheme on the iPhone, with the laptop inference server stopped and no network required after setup.**

Include:

- An iOS-only local Expo module and a typed JavaScript wrapper.
- Reuse of the current Pheme C ABI, Swift bridge and production XCFramework build.
- Import, identify, configure and load a local Whisper model once per engine lifetime.
- Transcribe a small saved WAV, then connect the current microphone recording flow.
- Local readiness, transcript review, structured failures, run IDs and observable metrics.
- Explicit lifetime, concurrency, interruption and cancellation behavior.

Later milestones add durable benchmark-run storage/upload and local reasoning. The first milestone does not claim that the complete incident-reporting workflow or benchmark pipeline is ready.

## Existing implementation to reuse

| Existing component | Use |
| --- | --- |
| `ios-app/` | Product interface, Expo configuration, PCM/WAV recording, recording recovery and SQLite |
| `pheme-va/include/pheme_va.h` | Versioned engine/configuration/transcription/observer API |
| `pheme-va/hosts/ios/Sources/NativeBridge.swift` | FFI ownership, serial inference queue, typed results/errors and independent observation |
| `pheme-va/hosts/ios/Sources/PhemeService.swift` | Reference behavior for model import, saved setup, busy state, observation and memory pressure |
| `pheme-va/scripts/build-ios.sh` | Device/simulator CPU Whisper XCFramework and build metadata |
| `pheme-va/hosts/ios/Tests/` | Existing fake-backed bridge and lifecycle checks |
| `pheme-va/scripts/ios-model-fixture.sh` | Pinned small model/audio smoke-test assets; optional, externally downloaded |
| Go experiment/metric APIs and SQLite | Later upload destination and dashboard queries |

The Expo client currently sends WAV/text to desktop Pheme through Go. The Swift host is a separate local-transcription prototype. Neither is already the combined deployment above.

## Step 1 — Establish a reproducible native development build

1. Resolve the mobile dependency/lockfile mismatch found during review (`react-native-worklets` missing from the lockfile). Verify compatible versions against the project's installed Expo SDK; avoid an unrelated SDK upgrade.
2. Make a clean `npm ci` and `npm run typecheck` pass.
3. Set up an Expo development build that can contain custom Swift/Rust code. Expo Go cannot host this custom native module.
4. Select/install full Xcode with an SDK supporting the target phone's actual iOS version. Check simulator availability, signing, device trust and Developer Mode.
5. Record the working Node/npm, Expo, Rust, Xcode/SDK and CocoaPods versions. Reuse existing pinned Rust/iOS build settings where compatible; do not change pins without a demonstrated build requirement.

Current host evidence: `xcode-select -p` resolves to `/Library/Developer/CommandLineTools`, and `xcodebuild -version` cannot run with that selection. A full workspace build also hit missing C++ standard-library headers. Installing/selecting a working native toolchain is a build gate, not evidence of a Pheme logic defect.

**Acceptance:** a clean checkout can build and install the current Expo app as a native development build on the target phone, before adding inference.

## Step 2 — Link the existing Pheme library into a local Expo module

Create the app-specific module under `ios-app/modules/pheme-native/` using the Expo module generator compatible with this app. Register iOS support only. Keep Expo-facing Swift code here; Rust engine/audio/transcription logic stays in `pheme-va/`.

- Package the production CPU Whisper device/simulator slices using the existing build script.
- Use the module's podspec to link the XCFramework and the system libraries already required by the Swift host.
- Keep generated native projects and binary/model artifacts out of Git. Check in module sources, configuration, artifact preparation instructions and build validation.
- Make a missing/mismatched XCFramework fail with an actionable setup error. It must not silently link a test or feature-disabled build as production inference.
- Reuse `NativeBridge.swift` through a shared source location or an explicit shared-source packaging rule. Keep it Foundation/FFI based and independent of Expo and SwiftUI; avoid two diverging copies.
- Verify both the Expo module and the standalone Swift host still resolve that shared bridge. Run the existing project path checks after any relocation.

**Acceptance:** a native smoke call returns ABI/capability information through JavaScript on simulator and device. Production archives contain no fake/test symbols.

## Step 3 — Expose a small native contract

Proposed operations; final names should follow the generated module conventions:

| Operation | Behavior |
| --- | --- |
| `inspect()` | ABI/schema version, capabilities, readiness, effective configuration and active run |
| `importModel(fileUri, identity)` | Copy a selected file into app-owned model storage and validate its expected identity/checksum |
| `listModels()` | Validated imported model identities and availability |
| `configure(settings)` | Load/change the selected model, language, vocabulary hints, threads and bounded audio limit |
| `transcribeFile(fileUri)` | Decode a saved app-owned WAV and return the existing run-correlated transcription result |
| `close()` | Stop admission and release only after all native operations/observation have settled |

Use the current synchronous C ABI behind the existing serial native execution queue. Async JavaScript calls must not run decoding, model loading or inference on the main thread. Keep one long-lived engine; do not reload weights for every turn.

For file input, decode the existing WAV format through native audio APIs into the float PCM accepted by `pheme_va_transcribe_v1`. Preserve actual rate/channel information; Pheme retains normalization, gating and transcription ownership. Bound file size, frame/sample count and duration before allocating a full PCM buffer. Pass file URIs through the bridge instead of base64 or JavaScript arrays of audio samples. If native decoding cannot preserve the required contract, evaluate a small additive WAV C-ABI entry point that reuses core decoding rather than writing a second WAV parser.

Return stable errors and preserve Pheme run IDs. Keep configuration and result schema versions explicit. Model metadata must distinguish the actual imported artifact from a user-entered display name.

Lifecycle rules:

- At most one model-changing/inference operation at a time; overlapping admission returns busy.
- Exactly one native owner drains metrics and fans out each original batch to consumers. Report observer overflow/gaps.
- Observation/status can remain responsive during inference. Keep observation buffers bounded.
- JavaScript reload, listener removal, navigation and rejected/cancelled promises do not permit freeing an engine still executing native work.
- The current FFI does not interrupt an active transcription. A UI cancellation discards its result and waits for settlement before allowing conflicting work or freeing memory; do not report an immediate native cancellation.
- Define background/interruption/memory-warning handling. Release or replace the model after active work settles.

**Acceptance:** fake-backed tests cover configuration, success, unavailable backend, invalid input, busy admission, observation ownership and safe cleanup/reload.

## Step 4 — Prove local inference with a small fixture

Add a small local setup/transcription route or development screen inside the Expo app. It should:

1. Import/select a model into app-owned storage.
2. Show model identity, readiness, effective thread count and audio limit.
3. Transcribe a selected short WAV through `pheme-native`.
4. Show text, run ID, processing time, status and available metric observations.
5. Explain missing/corrupt files and native failures without claiming readiness.

Use the checksum-pinned small English model/audio fixture already described by the iOS smoke script for the initial integration check. It is a pipeline fixture, not a selected production model. Download/import is setup; repeat inference must work offline. Model weights and recordings remain outside Git.

**Acceptance:** the iPhone 18 Pro returns a transcript with laptop Pheme and Go stopped. Verify a release-like standalone build/embedded JS bundle as well as development builds; Metro connectivity must not be confused with an inference dependency. Record the actual device/iOS/model/build identity and whether CPU-only execution was used.

## Step 5 — Connect the current microphone and review UI

- Reuse the existing Expo microphone capture and saved WAV path, then call local transcription instead of `startTurn` for this mode.
- Replace server readiness/pending-turn assumptions in local mode with native readiness/run state. Local startup must not depend on `/voice/inspect`.
- Preserve editable transcript review, recording recovery and explicit retries.
- Align the recording limit with the configured native cap. Expo currently permits 600 seconds, while the FFI allows at most 120 seconds. Stop at the local limit rather than record an unsupported file.
- Preserve the app-owned recording until local result persistence/recovery succeeds. Do not delete it using the old upload/SSE completion rule.
- Identify local transcription and any retained remote development chat as separate explicit modes. No silent desktop fallback when local inference fails.
- The local-only first milestone stops at an editable transcript; it must not display an active local reply model or call that full on-device conversation. Any retained desktop reply mode is labelled hybrid/remote.

**Acceptance:** recording, stop, local transcription and review work without a laptop. Denied microphone permission, silence, audio interruption, oversize audio, restart, retry, busy state and cancellation all have defined outcomes.

## Step 6 — Persist and upload on-device benchmark data (next milestone)

After the bridge contract is exercised with real outputs, extend the existing phone SQLite storage with versioned migrations for:

- Inference runs: stable run ID, device/install identity, actual model/configuration, timestamps, outcome, result and measurement boundary.
- Original metric batches: experiment/run/batch identity, exact payload and unavailable reasons.
- An upload outbox: pending/acknowledged state, retries and last upload failure.

Allocate stable client identities before upload and retain them across restarts. Persist a run before starting it and mark interrupted work visibly after relaunch rather than automatically rerunning inference. Import/export correlation must account for Pheme's admitted run ID rather than assuming it is caller-controlled.

Persist drained batches before considering observation delivery complete. A JavaScript event alone is not durable: design native retention/acknowledgement or a durable native spool so reload/disconnection cannot silently lose an already-drained batch. Define overflow limits and expose dropped measurements; do not claim crash-safe collection beyond what tests establish.

Define the phone-origin run registration/upload API against the existing Go experiment and metric models. Go's current create endpoint schedules desktop work, so a phone upload must not use it in a way that accidentally launches `PhemeRunner`. Reuse the metric schema and query/export APIs, adding an explicit external/on-device run path where needed.

On the laptop:

- Use one well-defined ingestion route for phone results and validate claimed experiment/run identifiers.
- Fix the competing destructive Pheme collectors and multi-case run-ID collisions identified in review; never rewrite unrelated batches into `phone_voice` experiments.
- Acknowledge only after successful persistent storage. Deduplicate identical retries and reject conflicting payloads without losing the phone's pending record.
- Keep actual device/model/runtime metadata and distinguish phone inference from desktop inference.
- Notify the existing dashboard after commits; correct its completion-event subscriptions and scope/source grouping.

**Acceptance:** local inference survives app restart as a stored run; metrics upload after laptop reconnection; a lost response can be retried without duplicate results; dashboard/export show the same actual phone run and unavailable values. Fault injection must cover a crash before/after persistence and acknowledgement. Laptop availability never controls whether local inference can run.

## Step 7 — Extend to local reasoning and incident workflow (later milestone)

Full deployment also needs a replaceable iOS-compatible in-process reasoning adapter and the authoritative Pheme incident workflow. The desktop llama.cpp child-process worker is not an iOS embedding solution. Resolve its native GGML compatibility/build boundary before choosing an implementation; do not silently move reasoning to the laptop and call it on-device.

Reuse Pheme-owned context, validated actions and future revision-bound report confirmation/storage/retrieval. React/Go must not acquire a second incident state machine. Model quality, resource use and local speech output availability need separate physical-device evaluation.

## Validation and delivery

Deliver reviewable changes in this order:

1. Mobile dependency/build repair and documented native development setup.
2. Local module/XCFramework link and lifecycle tests.
3. Model/file transcription bridge and offline physical-device acceptance.
4. Microphone/transcript-review integration.
5. Versioned phone persistence, acknowledged uploads and laptop dashboard integration.
6. Local reasoning and incident workflow after its native boundary is designed.

Run Expo type checking and meaningful native-wrapper/state tests; add mobile CI incrementally. Keep Rust tests local/fake backed by default; run formatting, Clippy and relevant workspace tests for Rust changes. Run existing Swift/iOS packaging and fake simulator checks when native sources/build packaging change. Validate Go changes with formatting, vet and package/integration tests. Real models and physical-device checks are explicit acceptance gates, not replaced by simulator fakes.

Do not commit generated iOS projects, models, XCFramework binaries, recordings or measurement artifacts. Preserve existing public Go routes and FFI symbols unless a separately documented implementation change requires otherwise.

## Decisions and prerequisites still to confirm

- Exact target iOS version and current Expo Go/development-build workflow.
- Working full Xcode selection, signing/team provisioning and ability to install on the phone.
- Available model storage and chosen initial test artifact; production model selection follows measurements.
- A test machine/build path supporting the target iOS version and current Expo SDK.
- Whether the next implementation request covers only steps 1–5 (native transcription) or also step 6 (persistent benchmark uploads).

No deployment performance, power, endurance or full on-device conversation claim is made by this plan.

## Primary references

- [Expo: local native modules](https://docs.expo.dev/modules/get-started/).
- [Expo: development builds and custom native code](https://docs.expo.dev/develop/development-builds/introduction/).
- [Expo: native-module registration](https://docs.expo.dev/modules/module-config/).
- [Existing native Pheme iOS setup and acceptance gates](../../pheme-va/hosts/ios/README.md).
- [Existing dashboard API contract](../api/benchmark-ui.md).
