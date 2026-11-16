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
- A shared foreground metrics collector, SQLite upload queue, server settings,
  retry/status controls and phone-origin Go ingestion without remote inference.
- **Results** opens the current or saved conversation, with overall processing,
  memory and thermal graphs, individual metric histories, and turn/operation
  detail. Benchmark **View results** uses the same saved observation archive.
  Reports and sharing use the displayed snapshot; a server is not required.

Attachments are staged only, not analyzed. Replies are development output, not
finalized incident reports. Structured confirmation/storage/retrieval remains
Pheme core work. Device resource/energy sampling and full offline/restart
acceptance remain pending. Foreground phone uploads were
verified on the paired iPhone on 2026-11-09; see the native verification record.

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
and your `ios.appleTeamId` there, then select the device when building. The
configured team preserves this workspace's previous signing selection.

Prebuild regenerates Expo's scene-based app host and pods. Repeat it after
config/plugin changes so an older ignored `ios/` project does not survive.

`native:bundle` reuses/downloads and verifies the pinned Whisper/Qwen weights,
then stages them as CocoaPods resources. No phone-side model download/import is
required. Reasoning bundles Qwen3-4B-Instruct-2507 Q4_K_M (Unsloth GGUF);
Whisper remains large-v3-turbo. The weights alone total about 4.12 GB; suitability for the phone's
memory/latency budget is not yet established. Use `npm run ios:release` for
embedded JavaScript and offline acceptance. Expo Go cannot run this module.

## Connect metrics over Wi-Fi

Rebuild/reinstall after the new Swift metadata API; Xcode export settings alone
cannot add the uploader to an older installed app.

1. Put the Mac and iPhone on a trusted LAN without client isolation. From `api/`,
   run `API_ADDR=0.0.0.0:8080 go run ./cmd/server`. Desktop Pheme is not required
   for phone ingestion.
2. From `web/`, run `npm run dev -- --host 0.0.0.0 --port 5173 --strictPort`.
   Open `http://localhost:5173/dashboard` on the Mac.
3. On the phone, open **On-device model settings**, then **Metrics sync**.
   Enter the Mac's current Wi-Fi address, for example
   `http://192.168.1.40:8080`, and choose **Save server and sync**.
   `localhost` on the phone refers to the phone, not the Mac.
4. Allow Local Network access when iOS asks. Keep the app foregrounded and
   complete a transcription or typed reply. The acknowledged count should
   increase and the dashboard should show `device_reported` observations.

For a new metrics database, `EXPO_PUBLIC_METRICS_URL` can supply the initial
server address when bundling/building JS. Saved settings take precedence after
initialization. No USB connection is required for HTTP uploads.

Uploads contain native timing events and model/device metadata, not audio or
conversation text. Pausing uploads retains local batches; offline batches retry
when the app is active again. CPU/RAM, GPU, battery and thermal sampling are not
implemented; power/energy remain unavailable. The API is unauthenticated:
do not expose it to the internet. See the [metrics contract](../docs/api/metrics.md)
for queue limits, idempotency and provenance.

See [native tests and acceptance](NATIVE_TESTING.md) for build gates, lifecycle,
model provenance and what still requires physical-device testing.
