# Incident Reporting Agent mobile app

Separate Expo/React Native implementation of the web voice-console flow. The existing `../web` app is untouched.

## Current implementation

- Chat-style incident-reporting screen with a large centered microphone action.
- Typed questions sent to the existing `/api/v1/voice/turns` SSE flow.
- Visible transcription and reply model readiness labels from `/api/v1/voice/inspect`.
- Transcription model and reply model readiness labels.
- Transcription is placed directly in the bottom input bar for editing; no separate review box is shown.
- The input-bar arrow approves and sends the edited transcript.
- User bubbles appear only after approval; streamed replies show animated dots while generating.
- The composer hides while a reply is generating and returns when the turn finishes.
- Pending server turns are recovered after app reload; conflicting new turns are blocked until the current one is finished.
- Native speech playback via `expo-speech`, including automatic speech for the first and subsequent completed replies.
- Microphone reports captured as mono 16 kHz PCM and sent as WAV bytes without constructing a React Native `Blob`.
- Multi-file selection with removable attachment chips.

Attachments are intentionally staged but not uploaded yet because the current Go API accepts text or WAV audio, not multipart files. The next backend change should add the multipart contract described in `../ios-app-draft/README.md`.

## Run

```sh
npm install
EXPO_PUBLIC_API_BASE_URL=http://YOUR_COMPUTER_IP:8080/api/v1 npx expo start --lan -c
```

For a physical iPhone, `127.0.0.1` points to the phone, not the development computer. Use the computer’s LAN IP and allow the API port through the firewall. An HTTPS deployed API is recommended outside the local network.

The mobile client captures mono 16 kHz PCM with `expo-audio`, switches the audio session back to playback mode, wraps the recording as WAV bytes, and sends it through the existing WAV transcription endpoint. The transcript is placed in the input bar for editing; tap the centered arrow to approve it. During reply generation the input bar is hidden, animated dots are shown, and the completed reply is spoken automatically through the phone’s native TTS. If the first reply is silent, verify that the phone has an installed system voice and is not muted. Multipart attachment upload remains pending because the Go API currently accepts JSON text or complete WAV audio only.

## Next implementation steps

1. Add multipart attachment upload after the backend contract is available.
2. Persist unsent drafts and conversation history deliberately, with a clear privacy setting.
3. Add unit tests for SSE parsing, turn state transitions, duplicate-send prevention, and attachment validation.
