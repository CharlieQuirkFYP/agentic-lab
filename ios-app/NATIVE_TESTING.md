# On-device Pheme: build and test

Status: native module, local WAV/microphone screen, transcript recovery and automated tests are implemented. A successful full Expo iOS native build and physical iPhone acceptance are still required. The current development machine selects Command Line Tools, not full Xcode. Passing JavaScript or host Swift tests does not establish that the Expo app has linked or run successfully on an iPhone.

The target is the user's iPhone 18 Pro; record its actual iOS version before selecting Xcode/SDK and signing settings. Expo Go cannot contain the custom Swift/Rust module. Use an installed development build, then a release-like standalone build for offline acceptance.

## Architecture and scope

```text
Expo local-transcription route
  -> PhemeNative local Expo module
  -> shared NativeBridge.swift -> Pheme Rust C ABI
  -> local CPU Whisper model
  -> saved local transcript/draft
```

There is no desktop inference fallback in this route. The original remote development chat remains available separately. The local route does not implement reasoning, incident finalization, power measurement, durable metric upload or a laptop exporter yet. Live metrics are bounded temporary diagnostics; local SQLite transcript persistence is not a benchmark outbox.

Native calls have one process-wide owner scoped to each JavaScript context. A stale context cannot close its replacement. Model loading and inference are serialized off the UI thread; status/metrics remain inspectable. Cancel discards a result but cannot interrupt the current native inference. The next model cannot load until prior native work settles. Backgrounding cancels result delivery; memory warnings request engine shutdown after settlement. Closing the route releases its engine after outstanding operations finish.

## Tests without Xcode, model weights or a device

From `ios-app/` with Node 22.18+:

```sh
npm ci
npm run typecheck
npm test
npm run test:native
```

`test:native` needs macOS and a working Swift/macOS SDK (Command Line Tools can suffice). It compiles the actual Foundation runtime, shared bridge types and AVFoundation WAV decoder with an injected fake engine. It covers checksum/import failures, saved setup, warm engine reuse, WAV shape/interleaving and duration validation, overlapping work, cancellation, delayed destruction, JS-context replacement, corrupt model recovery and path/symlink rejection. JavaScript tests cover native response errors, missing native builds, recording cleanup ordering, SQLite schema migration, applied settings and production artifact checks.

To additionally test the actual Rust-to-Swift C boundary without Whisper weights/native Whisper compilation:

```sh
cd ../pheme-va
CARGO_NET_OFFLINE=true bash scripts/test-ffi-swift.sh disabled test
```

Fetch locked Cargo dependencies first if the cache is empty. The optional script arguments select test variants; its original no-argument invocation still tests disabled, real Whisper and fake-backed builds. Fake entry points are confined to isolated test artifacts and must never ship in the app. The existing `bash scripts/test-ios-project.sh` check additionally requires XcodeGen; CI installs it before native project/build checks.

## Prepare the real native app

1. Install/select full Xcode and its simulator SDK. Set up device trust, Developer Mode and signing for the actual iPhone. Use an SDK that supports the phone's installed iOS. The existing Pheme iOS scripts pin Xcode 26.0/Rust 1.99.0 and target iOS 17+; the target phone's compatibility is an acceptance check.
2. Build the CPU Whisper library from the repository:

   ```sh
   cd pheme-va
   bash scripts/build-ios.sh whisper
   cd ../ios-app
   npm ci
   npm run native:prepare
   npm run native:check
   npm run ios
   ```

3. Select the device and development signing team as needed. Set a team-owned bundle identifier in `app.json` if required. `npm run ios` prepares native inputs before calling `expo run:ios --device`. After installation, use `npx expo start --dev-client` for development.
4. Rebuild the native app after Swift/Rust/native dependency changes. JS Fast Refresh cannot replace native code. Run `native:prepare` again after changing shared Swift sources. Rebuild the XCFramework after changing Rust/header/Cargo inputs.

The preparation script copies only generated artifacts into ignored `modules/pheme-native/ios/Frameworks/` and `Generated/`. The canonical shared Swift source remains in `pheme-va/hosts/ios/Sources/`; do not edit generated copies. Preparation verifies both library slices, production symbol checks, header/Cargo fingerprints and library checksums against build metadata. Pod installation also checks source/artifact freshness and fails with setup instructions if inputs are missing/stale. This intentionally prevents a successful-looking development build with a disabled/fake backend.

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

1. Open **On-device transcription** from the existing chat header.
2. Keep the default tiny.en test-model name/checksum and choose **Import model file**. Import copies and hashes the selected file into app-owned storage. For another model, supply its trusted checksum and name.
3. Select the imported model and choose **Load model / apply settings**. Confirm ready state and the actual artifact hash. Recording stays disabled when form settings differ from the applied native settings.
4. Choose **Transcribe a saved WAV** and select `jfk.wav`. Check the text, run ID, model hash and available timings. Use **Record on this iPhone** to test microphone capture and editable transcript review.
5. Edited drafts and run/artifact metadata persist in the phone's `local_transcriptions` table in `voice-recordings.db`. Saved microphone WAVs are retained until result persistence succeeds, with a separate `local` purpose so the remote chat does not upload them accidentally.

The native limit is 1–120 seconds, bounded by the applied configuration. Model import is capped at 8 GiB; that is an input bound, not a claim that the device can load an 8 GiB model. WAVs are capped at 32 MiB, supported channel/rate/sample limits and the configured duration before allocating decoded PCM. No full PCM arrays/base64 traverse the JS bridge.

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

`.github/workflows/ios-app-ci.yml` runs clean npm installation, type checks, JavaScript/storage tests, fake-backed Swift/runtime/decoder tests and iOS JS bundling. A separate job builds the production XCFramework, generates the Expo iOS project, installs pods and compiles the complete simulator app with signing disabled. It downloads no model weights. Build diagnostics are retained as artifacts.

The first successful native-link CI job establishes compilation/linking only. It does not replace the real-model/device checks above or establish model quality, memory endurance, power, thermal behavior, native microphone correctness or full local conversations.

Follow-on work: persist original metric batches with a crash-safe outbox/acknowledgement boundary, define phone-origin run ingestion in Go, correct desktop collectors/dashboard attribution, and implement an iOS-compatible local reasoning adapter. Those are tracked in the [deployment plan](../docs/plans/expo-on-device-pheme-plan.md).
