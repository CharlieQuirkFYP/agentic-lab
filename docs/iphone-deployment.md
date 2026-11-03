# Bundled iPhone deployment

There is one iPhone frontend: **Expo in `ios-app/`**. Its Swift module in
`ios-app/modules/pheme-native/ios/` links Pheme's Rust transcription core and
isolated native reasoning framework. The former SwiftUI app under
`pheme-va/hosts/ios/` and its deployment scripts have been removed.

```text
Expo chat -> Swift native bridge -> Pheme core / native adapters -> bundled models
```

Go, the desktop HTTP host and Expo Go are not used for local inference. The web
dashboard remains separate. The native app is not a cross-app background service.

## Prerequisites

Use an Apple Silicon Mac with full Xcode, an installed iOS simulator runtime,
CMake, CocoaPods, Node 22.18+, Rust, and the iOS Rust targets. The app targets iOS 17 or later.
The native scripts default to Rust 1.99.0; an explicit
`PHEME_IOS_RUST_TOOLCHAIN` override selects another toolchain. Keep the selected
toolchain and Xcode/SDK versions with any benchmark results.

For a physical device, sign into Xcode with an Apple development account,
connect and pair the phone, enable Developer Mode, and keep it unlocked for
installation/launch. Get the team ID from Xcode's account settings, not the
parenthesized individual certificate identifier.

```sh
xcodebuild -version
xcrun devicectl list devices
security find-identity -v -p codesigning
brew install cmake cocoapods
```

All following commands run from `ios-app/`. No changes to Git signing
configuration are needed.

## Build And Run

Build the native frameworks, verify/download the pinned models and install the
Expo app on a selected device:

```sh
npm ci
PHEME_IOS_RUST_TOOLCHAIN=stable npm run native:build
npm run native:bundle
npx expo prebuild --platform ios
npm run ios
```

Prebuild regenerates the Expo host and installs pods. Run it again after
config/plugin changes, including when migrating an older generated `ios/`
directory. Native host files are generated, not a second source frontend.

The current weights are Whisper large-v3-turbo, 1,624,555,275 bytes, and
Qwen2.5-1.5B-Instruct Q4_K_M, 1,117,320,736 bytes. The model payload alone is
2,741,876,011 bytes. Downloading, staging, copying into build products, signing,
and installing take time and require additional disk space. These are
development baselines, not validated latency/memory/thermal choices.

Set the team-owned bundle identifier in `app.json`, then select your signing
team/device in Expo/Xcode. For embedded JavaScript and offline acceptance:

```sh
npm run ios:release
```

For a simulator, after preparing models/frameworks:

```sh
npx expo run:ios --configuration Release
```

Use ARM64: the native packages contain device and ARM64 simulator slices only.
Rebuild native libraries after Rust/header/Cargo changes, and rebuild the app
after Swift or model resource changes. JS Fast Refresh cannot update them.

## Development IPA

Use the generated Expo workspace under `ios-app/ios/`, select a device target,
then Xcode Product > Archive and distribute for development. Old PhemePhone
IPAs are the removed SwiftUI app and must not be reused for Expo acceptance.
TestFlight/App Store distribution and model licence compliance remain separate.
Generated native projects, libraries, weights and app products stay ignored.

## Validation

From `ios-app/`:

```sh
npm run typecheck
npm test
npm run test:native
npm run native:check
ruby scripts/check-phone-app.rb /absolute/path/to/IncidentReportingAgent.app
```

The last command checks a finished bundled **Release** app: embedded Expo JS,
Expo scene lifecycle, microphone permission, dynamic reasoning, native linkage,
model sizes/hashes and role. It does not install anything. Prebuild wires the
SDK-provided Expo scene delegate, required for iOS 27, without a custom native UI.
See [native testing](../ios-app/NATIVE_TESTING.md)
for detailed coverage and remaining gates.

Verify a physical phone in the foreground with network access disabled:

1. Confirm both bundled models load, without importing or downloading files.
2. Record speech, stop, and inspect the local transcript.
3. Correct the transcript in Expo's input bar and approve it for a local reply.
4. Repeat turns, reset, cancel, interrupt recording, and restore after memory warnings.
5. Record cold/warm latency, peak memory, model checksums, device/OS/build settings,
   and sustained thermal behaviour. Power/energy remain unavailable without
   actual instrumentation.

Local transcription and conversational replies are development capabilities.
Structured incident revision confirmation, final report saving/retrieval,
complete dashboard integration for native phone metrics, and speech-output
acceptance are separate gates. Do not label a generated reply as a saved report
or a successful build/install as end-to-end workflow acceptance.

## Expo Verification Record

On 2026-11-03, Xcode 27.2 built the complete ARM64 simulator Release app with
both rebuilt native frameworks and the verified bundled weights. The finished-app
checker passed, and the app installed and launched on the iPhone 18 Pro iOS 27.0
simulator. The existing Expo chat displayed both models ready; native logs showed
the core-default 4,096-token reasoning context.

This verifies Expo startup and model loading, not a completed microphone/reply
turn or physical-phone deployment. See the [test record](../ios-app/NATIVE_TESTING.md#local-verification-record-2026-11-03)
for passing checks, the unrelated desktop CLI test failure and remaining gates.

## Historical SwiftUI Build Record

The following record is retained for provenance only. It predates the Expo
consolidation and does **not** establish Expo build, installation or inference
acceptance. The referenced generated artifacts may still exist locally; their
frontend and deployment scripts have been removed.

On 2026-11-01, using Xcode 27.1 (27A9275):

- ARM64 simulator and signed ARM64 device Release builds passed.
- Both finished bundles passed model SHA-256/size, role, native embedding, and
  portable-linkage validation; the device app passed deep code-sign verification.
- Signed archive and Xcode development IPA export succeeded. The IPA passed
  `unzip -tq`; its SHA-256 is
  `9f1bce590d64cf7154bc0787f85bdf182926945b929fcf83f1f07ed9aca1af69`.
- The app extracted from the final IPA passed the same model/native/role checks
  and deep code-sign verification, not just archive-stage validation.
- The isolated simulator deployment installed and launched Pheme (PID 15246).
  Its primary display showed the bundled Whisper selection, "Ready for local
  transcription," and reasoning "ready." Launch evidence is retained at
  `pheme-va/target/ios/deploy/simulator/launch.png`. This simulator reported a
  first-boot migration warning; microphone/reply interactions were not exercised.
- Exported bundle ID: `org.agenticlab.PhemePhone`; embedded framework:
  `PhemeReasoning.framework`; signing team: `4UDU99RT93`.
- The exported development provisioning profile expires **2026-11-08**.
  Rebuild/re-provision after expiry. The certificate expires 2027-11-01.
- Physical iPhone installation was interrupted by a CoreDevice/Mercury
  connection loss. The connected iPhone 18 Pro (iOS 27.0.1) became unavailable;
  fresh installation and physical launch were not confirmed.
- Offline deployment tests, shell/Ruby/plist syntax checks, Rust formatting,
  Go package tests/`go vet`, and the incoming dashboard production build passed.
  Dashboard lint reported eight warnings and no errors.

Those simulator and archive runs used isolated generated projects and
completed with exit code 0. The now-removed deployment script passed its offline
archive regression checks, including rejection of an invalid exported IPA.
Keep native inference/voice/device acceptance separate from this packaging
record; neither the IPA nor build success establishes those results.
