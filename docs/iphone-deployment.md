# Bundled iPhone deployment

The native development app in `pheme-va/hosts/ios/` embeds Rust Whisper and an
isolated in-process reasoning framework. Both model weights and the incident
role are copied into the app bundle. Go, a desktop inference worker, Expo Go,
and a network API are not needed to run these native adapters on the phone.
The optional paired desktop monitor is observation/control only.

`ios-app/` is a different Expo client introduced on `main`: it sends voice/text
turns to the desktop Go/Pheme API. Installing that client does not make its
inference on-device. The benchmark dashboard in `web/` stays a desktop research
application; it is not embedded in the native app.

## Main Integration

Fetched and merged `main` at `430d8e8` into the active feature branch on
2026-11-01, producing merge commit `dde8f97`. Existing uncommitted Pheme work
was preserved without stashing or conflict resolution. The main updates add:

- Benchmark dashboard pages for testing, experiment details, comparisons, and downloads.
- An API-backed Expo iPhone voice client and improved standalone web recording.
- Pheme-backed benchmark cases, persisted voice history/metrics, phone-voice
  experiment entries, and event-driven dashboard updates.
- LAN startup instructions and architecture diagrams.

The deployment changes remain uncommitted; no branch was pushed.

## Prerequisites

Use an Apple Silicon Mac with full Xcode, an installed iOS simulator runtime,
CMake, XcodeGen, Rust, and the iOS Rust targets. The app targets iOS 17 or later.
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
brew install cmake xcodegen
```

All following commands run from `pheme-va/`. No changes to Git signing
configuration are needed.

## Build And Run

Build the native frameworks, verify/download the pinned models, generate the
bundled-model project, build a Release app, validate the finished bundle, and
install/launch it on an available iPhone simulator:

```sh
bash scripts/deploy-phone.sh simulator
```

The current weights are Whisper large-v3-turbo, 1,624,555,275 bytes, and
Qwen2.5-1.5B-Instruct Q4_K_M, 1,117,320,736 bytes. The model payload alone is
2,741,876,011 bytes. Downloading, staging, copying into build products, signing,
and installing take time and require additional disk space. These are
development baselines, not validated latency/memory/thermal choices.

For a physical iPhone:

```sh
export PHEME_IOS_DEVELOPMENT_TEAM=YOUR_TEAM_ID
export PHEME_IOS_DEVICE_ID=YOUR_PHONE_UDID
bash scripts/deploy-phone.sh device
```

An optional `PHEME_IOS_BUNDLE_ID=org.yourteam.PhemePhone` avoids an identifier
already owned by another team. An optional `PHEME_IOS_SIMULATOR_ID` selects a
specific installed simulator.

When `build-phone.sh` and both native frameworks are already prepared:

```sh
bash scripts/deploy-phone.sh simulator --use-prepared
bash scripts/deploy-phone.sh device --use-prepared
```

`--use-prepared` skips model/native rebuilding but regenerates the bundled app
project and validates the finished app. It is appropriate for Swift/UI changes;
rebuild the relevant native frameworks after Rust changes. `--build-only`
suppresses simulator/device installation and launch.

The deployment script deliberately forces ARM64 because the packaged native
libraries contain ARM64 device/simulator slices, not an Intel simulator slice.
It validates microphone permission, reasoning embedding, portable dynamic
dependencies, private native symbols, both model sizes/SHA-256 values, the
incident role, and model notices. Device products additionally pass code-signing
verification before installation. Failed builds or validation never install.

## Development IPA

Export a signed archive and development IPA with the selected team:

```sh
bash scripts/deploy-phone.sh archive --use-prepared
```

The archive is `target/ios/deploy/archive/PhemePhone.xcarchive`; the IPA is
`target/ios/deploy/archive/export/PhemePhone.ipa`. Export uses automatic signing
and Xcode's `debugging` distribution method. Development provisioning restricts
installation to authorized devices and may expire. This is not TestFlight or
App Store distribution; review and include full required model licence texts
before redistribution.

All generated apps, archives, native libraries, models, and deployment logs stay
under ignored `pheme-va/target/`. Do not add them to Git. Logs, tool versions,
signing details, and model validation output are retained in
`target/ios/deploy/<mode>/`. Archive export additionally checks IPA ZIP integrity
and writes `ipa-validation.txt` and `ipa-sha256.txt`.
Generated projects use `target/ios/phone-<mode>/project/`, separately from the
ABI/model-test project, so simulator tests do not overwrite a deployment build's
project references.

## Validation

The offline deployment regression suite needs only Node and Ruby, and does not
download models or contact Apple:

```sh
node scripts/test-deploy-phone.mjs
bash -n scripts/deploy-phone.sh
ruby -c scripts/check-phone-app.rb
```

It covers simulator installation, explicit physical-device/team selection,
archive export, paths with spaces, build-only mode, corrupt model rejection,
native dependency/symbol leaks, and build/sign/install/export failure gates.
The iOS workflow runs it alongside existing native packaging and Swift checks.
Offline fake tools do not establish real device acceptance.

Verify a physical phone in the foreground with network access disabled:

1. Confirm both bundled models load, without importing or downloading files.
2. Record speech, stop, and inspect the local transcript.
3. Select **Use transcript**, correct it, and send the reviewed text for a local reply.
4. Repeat turns, reset, cancel, interrupt recording, and restore after memory warnings.
5. Record cold/warm latency, peak memory, model checksums, device/OS/build settings,
   and sustained thermal behaviour. Power/energy remain unavailable without
   actual instrumentation.

Local transcription and conversational replies are development capabilities.
Structured incident revision confirmation, final report saving/retrieval,
complete dashboard integration for native phone metrics, and speech-output
acceptance are separate gates. Do not label a generated reply as a saved report
or a successful build/install as end-to-end workflow acceptance.

## Local Build Record

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

The final simulator and archive runs used isolated generated projects and
completed with exit code 0. The current script passes shell syntax and offline
archive regression checks, including rejection of an invalid exported IPA.
Keep native inference/voice/device acceptance separate from this packaging
record; neither the IPA nor build success establishes those results.
