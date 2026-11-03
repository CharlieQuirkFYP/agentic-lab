# Pheme iPhone development host

This host embeds the portable Rust transcription engine through the C ABI. An opt-in bundled development build also embeds the existing Qwen reasoning adapter through a separate dynamic framework. The phone owns the models, audio capture, settings, and execution. Core transcription metrics flow out through one observer; no metrics-ingestion API is provided. Reasoning displays load/reply timings separately, not a full metrics batch.

The implementation is present, but full physical-device acceptance is **not yet complete**. The initial transcription app has been launched on a physical iPhone by the developer; model-backed microphone turns, both-model operation, lifecycle and resource checks below remain required. Full Xcode 27.1 and iOS simulators are now installed locally. CI still pins Xcode 26.0.

## Build and simulator checks

Use an Apple Silicon Mac with Xcode 26.0, its iOS simulator runtime, CMake, XcodeGen, and Rust 1.99.0. The minimum app deployment target is iOS 17.0. CI selects these Rust/Xcode versions and records the actual SDK/tool versions and library checksums.

The packaging script reuses the selected Rust toolchain when installed and installs it with the minimal profile only when unavailable. It also installs only missing iOS targets and `llvm-tools-preview`, then uses that toolchain's `llvm-nm` to inspect exported symbols. The default remains Rust 1.99.0; another installed version is not silently substituted. An explicit `PHEME_IOS_RUST_TOOLCHAIN` override selects a different toolchain, but is outside the pinned CI configuration. Missing prerequisites and uncached Cargo dependencies still require network access.

Xcode's older `nm` cannot reliably read the LLVM bitcode bundled into Rust 1.99 archives (`Unknown attribute kind`); this inspection failure does not establish that the native library cannot link. Keep the subsequent device link and simulator tests as the compatibility gates.

From `pheme-va/`:

```sh
brew install cmake xcodegen
bash scripts/build-ios.sh whisper
bash scripts/test-ios.sh whisper
```

Select full Xcode as the active developer directory before building. The package is written to `target/ios/whisper/PhemeVA.xcframework`. It contains ARM64 device and ARM64 simulator static libraries, the C header, and the Clang module map. The Swift project links the package with Accelerate, C++, Security, SystemConfiguration, and libresolv.

The [iOS workflow](../../../.github/workflows/pheme-va-ios-ci.yml) runs three independent variants:

| Variant | Purpose |
| --- | --- |
| `whisper` | Real CPU Whisper build, Swift linkage, unloaded/error/lifetime checks. |
| `disabled` | Stable v1 ABI without an adapter; legacy Whisper-only symbols are absent. |
| `test` | Successful core transcription and result/metrics ownership using an isolated fake. |

The test variant uses the private `pheme_ffi_test` compile configuration in a separate Cargo target directory. It adds a test constructor only to its test header. Production archives are checked for leaked test symbols, and only the production Whisper XCFramework is uploaded. Do not distribute the test artifact.

The workflow builds the Swift app for the device slice without signing and runs XCTest on the simulator. This catches C header import and final native linkage errors that a Rust library build alone cannot catch.

Cargo dependencies are fetched once with `--locked` before both slice builds run with `--frozen` (offline, with no lockfile changes). CI caches dependency downloads and allows five Cargo network retries. This reduces repeated downloads but cannot repair a persistent runner DNS outage; a failure resolving `index.crates.io` is still a dependency-fetch failure, not a Whisper compile failure. Rerun after connectivity recovers.

Build metadata is written before fetching/compiling, and CI retains `build.log`, available symbol listings, metadata, and XCTest results even after build failures. `bash scripts/test-build-ios.sh` exercises orchestration and failure gates with offline fake tools on Linux/macOS; it does not replace native iOS checks.

XcodeGen uses `target/ios/<variant>/project` as both its project root and output directory. The spec's source, asset, and Info.plist paths are relative to that root. Setting only `--project` leaves some references relative to the spec directory and can produce a duplicated `target/ios/target/ios` framework path. The test script validates generated references and requires any referenced assets directory to exist before starting the simulator. `bash scripts/test-ios-project.sh` tests real XcodeGen output for clean checkouts, all variants, explicitly enabled assets, missing fixtures, and invalid references; it requires XcodeGen and macOS `plutil`, but no full Xcode, models, or simulator.

Relevant pushes to `feat/pheme-va-ios-ffi` and pull requests targeting `main` run the workflow automatically. A manual `workflow_dispatch` trigger is also defined, with an optional `model_test` input. GitHub requires the workflow file on the default branch before manual dispatch is available; until then, push this feature branch to run the checks. Once available, use **Actions → Pheme VA iOS CI → Run workflow**, select the branch, and optionally enable the model test.

For the optional real simulator transcription job, manually dispatch the workflow with `model_test=true`. It downloads:

- `ggml-tiny.en.bin`, 77,704,715 bytes, from Hugging Face `ggerganov/whisper.cpp` revision `5359861c739e955e79d9a303bcbc70fb988958b1`; SHA-256 `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f`.
- The upstream whisper.cpp `v1.7.6/samples/jfk.wav` fixture, 352,078 bytes; SHA-256 `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`. This is JFK's public-domain US government speech, used by upstream as a transcription sample.

Both are checksum-verified under ignored `target/ios/assets/`. With `PHEME_IOS_MODEL_TEST=1`, `test-ios.sh whisper` requires both files and selects `project-model-test.yml` to copy the assets folder into the test bundle. Ordinary runs use `project.yml` without an assets resource, even when downloaded fixtures are present. XcodeGen's `optional: true` does not reliably omit a missing `type: folder` resource from Xcode's copy phase. Model/audio paths are passed through `PHEME_VA_TEST_MODEL` and `PHEME_VA_TEST_AUDIO`. PR checks never download weights.

On a Mac with only Command Line Tools, `bash scripts/test-ffi-swift.sh` checks native macOS Swift linkage and ABI behaviour for the same three FFI variants. It disables GGML host CPU auto-detection and uses `armv8-a`, matching the iOS CPU baseline; the upstream SVE probe was observed hanging on the development Mac. For workspace tests on an affected Mac, use `GGML_NATIVE=OFF GGML_CPU_ARM_ARCH=armv8-a cargo test --workspace`. These are additional local checks; they do not replace the iOS pipeline.

Local validation on 2026-10-22 passed formatting, workspace Clippy, the complete Rust suite with one test thread (305 passed, 6 intentionally ignored), both FFI feature variants (7 / 8 tests), and all three macOS Swift/C integration variants. The full-suite command was `GGML_NATIVE=OFF GGML_CPU_ARM_ARCH=armv8-a cargo test --workspace -- --test-threads=1`. Earlier parallel runs intermittently failed existing TUI fixtures; their serial pass is not evidence that the default parallel suite is reliable. See the [implementation plan](../../../docs/plans/pheme-va-ios-ffi-plan.md) for the validation record and remaining gates.

## Open and run the app

### Bundled ASR and reasoning app

From `pheme-va/`, explicitly opt into packaging both models:

```sh
PHEME_IOS_RUST_TOOLCHAIN=stable bash scripts/build-phone.sh
open target/ios/whisper/project/PhemePhone.xcodeproj
```

`stable` is an explicit local choice, not an update request. The script reuses
and verifies existing files, or downloads missing files using the existing
checksum-verified downloader. It bundles Whisper large-v3-turbo (1,624,555,275
bytes) and Qwen2.5-1.5B-Instruct Q4_K_M (1,117,320,736 bytes), plus the incident
role instruction. These are development baselines, not validated phone
selections. Allow roughly 2.74 GB for model files alone and additional device
storage for installation. Large Whisper may exceed an individual phone's
memory or latency budget; simultaneous loading requires device acceptance.

Weights stay ignored by Git under `models/` and `target/`. `--prepare-only`
verifies/downloads and stages resources without building. The app opens
read-only model paths inside its bundle, without duplicating weights into
Application Support. File import remains available for alternative Whisper
models. Resource bundling is opt-in; ordinary PR/simulator checks do not
include model weights or the reasoning framework.

The separate **ASR Model (Whisper)** and **Reasoning Model** controls discover
the bundled files and select defaults on first launch. Models load sequentially
off the UI thread using a shared native queue. Saved selections are restored.
Each model has its own load state and settings. Record and transcribe, tap
**Use transcript**, review/correct the text, then **Send to reasoning model**.
Typed text can also test reasoning without microphone input. **Reset
conversation** clears Pheme-owned successful history without reloading weights.
Native reasoning retains at most two successful pairs and 256 request IDs,
uses a 2048-token context and 128-token reply limit, and cancels at 120 seconds.
Cancellation is cooperative: native resources remain owned until inference
settles. Inactivity discards pending replies; memory warnings release models
after admitted work settles. Reapply/load settings afterward.

Qwen uses the existing `reply-native` CPU adapter through `reply-ffi`, packaged
as `PhemeReasoning.xcframework`. It is a signed embedded dynamic framework,
not a downloaded executable, child process or server. Its private GGML symbols
are not exported; the packaging script rejects a symbol leak. Never statically
merge its native objects into `PhemeVA.xcframework`. This avoids the incompatible
Whisper/llama GGML collision without changing the desktop worker.

Project generation retains the existing app Debug signing team and bundle
identifier when present. `PHEME_PHONE_TEAM` and `PHEME_PHONE_BUNDLE_ID` explicitly
override them. Select your Personal Team in Xcode if none was previously saved.
No App Store/TestFlight packaging is included. Review upstream model licences
and provide full required notices before distributing model-containing builds.

To regenerate this project without rebuilding native libraries:

```sh
PHEME_IOS_APP_MODELS=1 bash scripts/test-ios.sh whisper --generate-only
```

Optional real-model simulator checks (large weights and CPU inference):

```sh
PHEME_IOS_APP_MODELS=1 \
PHEME_VA_TEST_REPLY_MODEL="$PWD/models/reply/qwen2.5-1.5b-instruct-q4_k_m.gguf" \
bash scripts/test-ios.sh whisper
```

This checks that both models can load in the same process and Qwen can respond.
It does not establish microphone acceptance, device memory or energy efficiency.
Replies are development output; the app does not finalize/save incident reports,
run continuously in the background, or synthesize spoken replies.

### Transcription-only app

After packaging, generate the project from `pheme-va/`:

```sh
bash scripts/test-ios.sh whisper --generate-only
```

Open `target/ios/whisper/project/PhemePhone.xcodeproj` in Xcode. For a physical iPhone, select a signing team and an appropriate bundle identifier, then build/run the `PhemePhone` scheme. There is no TestFlight upload or App Store distribution automation yet.

1. Provision a Whisper GGML model in Files on the phone.
2. Tap **Import Whisper model**. The app copies it into its own Application Support model directory.
3. Select language/dictionary hints, thread count, audio limit, and metrics collection.
4. Tap **Apply settings and load**. Settings are saved only after configuration succeeds.
5. Tap **Record speech**, then **Stop and transcribe**.

Startup restores saved settings and loads an available saved model. Missing/corrupt models leave a recoverable setup state. Changing language, dictionary, duration, or metrics does not reload a ready model; changing model or thread count reloads it. A replacement unloads the previous model first to limit peak memory.

This is a foreground development app. App inactivity, interruptions, and route changes stop recording and discard unwanted results. A cancelled UI result does not free native resources during inference. Memory warnings request model release after current work settles. The user can apply the saved setup again afterward. Always-on background service behaviour is outside this implementation.

## ABI and ownership

The authoritative declarations are in [pheme_va.h](../../include/pheme_va.h).

- `pheme_va_create` creates an unloaded handle; `pheme_va_abi_version` returns 1.
- `pheme_va_configure` accepts bounded UTF-8 JSON; `pheme_va_inspect` returns effective configuration, compiled capabilities, phase, active run, and last failure.
- `pheme_va_transcribe_v1` accepts finite interleaved float samples and returns `{schema_version, run_id, result}`.
- `pheme_va_close` stops admission; a busy response means work is still settling. The caller joins all outstanding calls before `pheme_va_free`.
- Observer creation/drain/inspection/free have an independent lifetime. An observer may outlive its engine.
- Every non-null output string, including error JSON, must be released exactly once with `pheme_va_string_free`.

Configuration example:

```json
{
  "schema_version": 1,
  "model_path": "/phone/app-owned/path/model.bin",
  "language": "en",
  "dictionary": ["KLASS", "west entrance"],
  "threads": 4,
  "max_audio_seconds": 120,
  "metrics_enabled": true
}
```

The limits are 16 KiB configuration JSON, a 4096-byte model path, 1–16 threads, 1–120 seconds, and 128 dictionary terms with a total prompt of at most 2000 characters. Audio accepts 8–192 kHz, 1–8 channels, complete frames, and at most 16,777,216 samples. Shape/duration limits are checked before copying the input. Output JSON is capped at 4 MiB.

Stable codes are 0 success, 1 invalid input, 2 backend failure, 3 busy, 4 unavailable, 5 unsupported, 6 closed, and 7 contained Rust panic. Destruction must not race any handle call, including inspection. Native C/C++ crashes, invalid pointers, and allocation aborts are outside recoverable Rust panic handling.

The legacy constructor/dictionary/transcribe/metrics/free symbols remain available in Whisper-enabled builds with their original signatures. Their metrics drain shares the new observer's queue; do not run competing drain owners.

## Metrics

The FFI retains at most 512 events, drops the oldest on overflow, and reports a cumulative dropped-event count. The existing MetricEvent/MetricBatch schema and sequence fields are unchanged. `MetricsBatcher::new()` remains unbounded for existing consumers; only the new bounded constructor changes retention.

The service polls every 250 ms independently of the inference executor, drains once, and distributes the same batch JSON to its display and optional monitor. It retains at most 20 drain payloads and 4 MiB for local display. A future Go exporter should retain the exact payload and batch identity for retries. No Go uploader is included in this host.

Each admitted transcription has a generated run ID returned in its result, active snapshot, and admitted-operation failures. Its emitted events carry that ID. An experiment ID is optional: the Go metrics route can supply it from the experiment URL.

Metrics cover core timings, status, and model metadata. Device power, energy, temperature, CPU, RAM, and battery observations are not collected here and must not be presented as measured zero.

## Optional desktop monitor

From `pheme-va/` on the desktop:

```sh
cargo run -p cli -- monitor --bind 0.0.0.0:8765
```

In the phone's **Desktop monitor** section, enter `ws://<desktop-LAN-address>:8765` and the generated pairing code displayed by the CLI. The phone connects outward. The default CLI bind address is loopback; binding a LAN address explicitly enables remote access. This is a paired development connection for a trusted LAN; the built-in desktop listener does not provide TLS.

The monitor starts without local weights or a local inference worker. Its controls are:

| Key | Action |
| --- | --- |
| `m` | Select the next already-imported phone model. |
| `-` / `+` | Change the draft thread count. |
| `e` | Toggle draft metrics collection. |
| Enter | Send the draft; display the phone's acknowledgement or rejection. |
| `r` | Request a fresh snapshot. |
| `f` | Load the optional `--config path.json` settings into the draft. |
| `q` | Exit, leaving phone inference intact. |

The optional settings file uses phone model filenames, not desktop paths:

```json
{
  "modelFile": "imported-phone-model.bin",
  "language": "en",
  "dictionary": "KLASS, west entrance",
  "threads": 4,
  "maxSeconds": 120,
  "metricsEnabled": true
}
```

Transport messages have version 1 and types `hello`, `paired`, `inspect`, `configure`, `ack`, and `observation`. Observations carry effective snapshots, selectable phone models, settings, exact `batches_json`, a cursor, dropped-observation counts, and `history_available=false`. Messages are capped at 256 KiB. Slow connections may drop observations; oversized metric payloads are omitted with a visible gap count. Acknowledgements use a separate bounded queue.

Reconnect explicitly in the phone UI. The desktop clears pending drafts/commands and receives fresh phone state; configuration commands are never replayed onto a new connection. Observation history is not replayed. One desktop listener accepts one paired phone connection at a time.

## Acceptance still required

Record the first successful iOS workflow run before declaring the library integration gate passed. On a selected physical iPhone, verify:

- Fresh import/setup, saved restoration, missing/corrupt model recovery, and switching models.
- Offline local transcription, repeated microphone turns, and responsive UI/metrics.
- Microphone denial, interruption, route changes, inactivity, cancellation, and memory warnings.
- Monitor pairing, rejected/busy configuration, reconnect gaps, and continued phone inference after disconnect.
- Cold/warm latency, peak memory, and sustained thermal behaviour with device/OS/model checksum/build configuration recorded.

CPU Whisper is the required baseline. Metal/Core ML execution, energy/endurance, continuous background operation, spoken replies, and the full incident-report workflow require separate implementation or acceptance. The opt-in CPU reasoning host also needs physical-device acceptance.

The Swift host plus embedded Rust C ABI/XCFramework is the on-device boundary. Go and the desktop monitor are not required for phone inference. The desktop runtime retains its child-process reply worker; the mobile host instead uses the same native adapter through the isolated dynamic reasoning framework and core conversation validation. It does not embed the desktop TUI/server runtime unchanged.
