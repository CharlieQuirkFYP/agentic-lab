# On-device Pheme: build and test

Status: Expo owns the chat/settings UI and all Swift host code. Local transcription,
reasoning, bundled model preparation, transcript recovery and automated tests are
implemented. Local Release simulator linking, bundle validation, launch and
dual-model readiness passed; physical iPhone and interactive inference acceptance
remain pending. Old `PhemePhone.app`/IPA artifacts are from the removed SwiftUI
host, not evidence for this Expo build.

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
the full incident confirmation/storage/retrieval workflow. Metrics are temporary
diagnostics, not a benchmark outbox; power/energy measurements are unavailable.

Reasoning uses the core's `ConversationConfig::default()`: a 4,096-token context,
up to 512 output tokens, six retained successful pairs, 12,000 input characters,
4,000 output characters and temperature 0.2. These are shared development
defaults, not validated phone memory or latency limits.

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
SQLite migration and production framework/resource checks.

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
4. Select the device and development signing team as needed. Set a team-owned bundle identifier in `app.json` if required. `npm run ios` prepares native inputs before calling `expo run:ios --device`. After installation, use `npx expo start --dev-client` for development.
5. Rebuild the native app after Swift/Rust changes. Swift sources are compiled
   directly from the Expo module, not copied from another host. Rebuild both
   XCFrameworks after Rust/header/Cargo changes. Use `npm run ios:release` to
   embed JS, then test with Metro and laptop inference stopped.

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
The pinned Whisper large-v3-turbo/Qwen2.5-1.5B Q4_K_M weights total
2,741,876,011 bytes. Device memory and latency suitability remains unvalidated.

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

## Physical-device acceptance checklist

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
- Verify live metrics show actual run IDs and any dropped events. Do not report device power/energy as zero or assume desktop metrics describe the phone.

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

Follow-on work: structured incident workflow, durable metric outbox/ingestion,
phone-origin dashboard attribution, and measured physical-device acceptance.
The [deployment plan](../docs/plans/expo-on-device-pheme-plan.md) retains the
historical milestone breakdown; current ownership and commands are here.
