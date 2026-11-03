# Incident Reporting Agent mobile app

The iPhone app frontend is Expo/React Native. Swift integration belongs only to
`modules/pheme-native/ios/`; the separate SwiftUI development app has been removed.
The existing `../web` app is untouched.

## On-device inference

```text
Existing Expo chat / model settings
  -> PhemeNative Swift module
  -> PhemeVA (Rust core + Whisper) and PhemeReasoning (Rust + llama.cpp)
  -> local model files in the same app bundle
```

Chat now uses local native calls, not the desktop Go HTTP API. No silent remote
fallback exists. The former HTTP client source remains for reference, but is not
used by either route. Model settings also supports verified Whisper file import
and saved WAV testing. Physical-device dual-model acceptance remains required.

## Current implementation

- Chat-style incident-reporting screen with a large centered microphone action.
- Typed questions sent directly to native reasoning.
- ASR and Reasoning model readiness from the embedded engines.
- Transcription is placed directly in the bottom input bar for editing; no separate review box is shown.
- The input-bar arrow approves and sends the edited transcript.
- User bubbles appear only after approval; animated dots remain until the complete native reply returns.
- The composer hides while a reply is generating and returns when the turn finishes.
- Pheme owns bounded successful reasoning history; New chat resets it without reloading weights.
- Local recordings/transcripts and chat display history persist. Native reasoning context is not restored after process exit.
- Native speech playback via `expo-speech`, including automatic speech for the first and subsequent completed replies.
- Microphone reports captured as mono 16 kHz PCM, saved as WAV, and passed by file URI to Swift.
- Multi-file selection with removable attachment chips.

Attachments are staged only, not analyzed. Replies are development output, not
finalized incident reports. Structured confirmation/storage/retrieval remains
Pheme core work; durable metric export is not implemented here.

## Run

```sh
npm ci
PHEME_IOS_RUST_TOOLCHAIN=stable npm run native:build
npm run native:bundle
npx expo prebuild --platform ios
npm run ios
```

Run from `ios-app/` on an Apple Silicon Mac with full Xcode, CMake, CocoaPods and
the Rust iOS targets. `stable` explicitly selects the installed toolchain;
omitting it retains the pinned 1.99.0 default. Set your bundle ID in `app.json`
and select your development team/device when building.

Prebuild regenerates Expo's scene-based app host and pods. Repeat it after
config/plugin changes so an older ignored `ios/` project does not survive.

`native:bundle` reuses/downloads and verifies the pinned Whisper/Qwen weights,
then stages them as CocoaPods resources. No phone-side model download/import is
required. The weights alone total about 2.74 GB; suitability for the phone's
memory/latency budget is not yet established. Use `npm run ios:release` for
embedded JavaScript and offline acceptance. Expo Go cannot run this module.

See [native tests and acceptance](NATIVE_TESTING.md) for build gates, lifecycle,
model provenance and what still requires physical-device testing.
