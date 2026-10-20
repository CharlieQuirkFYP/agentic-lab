# iOS voice/chat app — product and technical draft

This is the approved product and architecture record for the separate `../../ios-app` implementation. It does not modify `../../web` or describe a committed release.

## Recommendation

Build the first iOS version with **React Native + Expo + TypeScript**, using an Expo development build rather than a browser wrapper. This reuses the team’s React/TypeScript skills while providing native microphone, speech, document picker, photo library, and share-sheet capabilities.

Use SwiftUI instead if the priority becomes deeply native audio processing, on-device inference, background recording, or the smallest possible production binary. For the current API-backed voice console, React Native is the faster and lower-risk choice.

## Product goal

Turn the current development voice console into a friendly assistant conversation:

- The person can type a question or record it.
- A transcript appears as an editable draft before anything is submitted.
- The person can rewrite, shorten, or correct the question and explicitly send it.
- The assistant reply streams into a normal chat bubble.
- Replies can be replayed aloud.
- Files can be attached to a draft and shown as removable chips before sending.
- Failed, cancelled, or interrupted work remains visible and can be retried deliberately.

The app remains API-backed. The existing reply model runs behind the Go/Pheme service; the first iOS release does not attempt to ship the GGUF model inside the phone.

## Proposed user experience

### Main conversation screen

1. The screen is titled **Incident Reporting Agent** and shows transcription/reply model readiness.
2. A centered microphone button is the primary action; it turns red and shows a stop icon while recording.
3. Recording is sent as mono 16 kHz WAV; the transcript is placed directly in the bottom input bar for editing.
4. **Start an incident report · Ask by typing or speaking** appears above the input bar. The input-bar arrow approves and sends the transcript; no separate review card is shown.
5. User bubbles appear only after approval. While the reply is generating, the composer hides and animated dots appear; the completed reply is spoken automatically with native TTS.
6. Pending turns are recovered after reload, and a new turn is blocked until the current one is completed or cancelled.
7. Attachments appear above the composer as removable chips but remain staged locally until multipart upload is added.

### Draft/review behavior

- Never send a transcription automatically.
- Preserve the original transcript while the edited question is shown as the draft.
- Disable send while an approval request is in flight.
- If the server reports different authoritative approved wording, replace the local draft and explain why.
- Keep drafts local to the current session unless persistence is explicitly added later.

### Friendly states

- Empty: “Ask by typing or speaking.”
- Recording: “Listening…” with a live timer.
- Reviewing: “Check your question before sending.”
- Generating: “Thinking…” with stop control.
- Offline/API unavailable: retain the draft and attachments; show “Couldn’t connect. Try again.”
- Attachment rejected: explain the exact file type/size reason without losing the question.

## Suggested folder layout

```text
agentic-lab/ios-app/
  app/                    # Expo Router screens
    (tabs)/chat.tsx
    settings.tsx
  src/
    api/                   # typed API client and SSE parser
    components/            # ChatBubble, Composer, ReviewCard, AttachmentChip
    features/chat/         # conversation state and turn lifecycle
    features/audio/        # recording, playback, permissions
    features/attachments/  # picker, validation, upload state
    storage/               # draft/session persistence only
    theme/                 # colors, spacing, typography
  assets/
  app.json
  package.json
  README.md
```

The product draft remains here as the design record; the approved initial implementation now lives in `../../ios-app`.

## API reuse and required backend work

The mobile client can reuse `GET /api/v1/voice/inspect`, `POST /api/v1/voice/turns`, the turn SSE events, `POST /api/v1/voice/turns/:id/submit`, `POST /api/v1/voice/turns/:id/cancel`, `GET /api/v1/voice/turns/:id`, and `POST /api/v1/voice/reset`.

Files are not currently part of the web voice contract. Add a backend contract before implementing the mobile attachment UI:

```text
POST /api/v1/voice/turns
Content-Type: multipart/form-data

text=<optional edited question>
audio=<optional recording>
files[]=<optional document/image>
```

The server should validate MIME type and size, assign attachment IDs, extract safe text/OCR where appropriate, and pass only approved attachment content to the reply service. Do not put API keys or model credentials in the app.

The mobile client uses a native PCM stream and wraps it as WAV before sending, so the existing WAV endpoint is sufficient for microphone input. Do not send unsupported M4A/WebM data to the current WAV endpoint.

## Attachment scope for v1

- Files app: PDF, TXT, DOCX, CSV, and common image formats.
- Photos/camera: optional, behind the same attachment flow.
- Show filename and size before sending.
- Enforce a visible client limit, but keep backend limits authoritative.
- Upload only after the person taps **Send question**.
- Show progress and allow removing an attachment before send.
- Never render uploaded HTML as executable content; display extracted content as text.

## Mobile architecture

- `expo-router` for navigation.
- `expo-audio` or the current Expo audio package for recording/playback, with a development build if native configuration is required.
- `expo-document-picker` and `expo-image-picker` for attachments.
- A typed `fetch` client for JSON plus an SSE parser that handles split UTF-8 frames and cancellation.
- Zustand or a reducer-based feature store for chat state; keep the turn lifecycle out of individual screen components.
- `expo-secure-store` only for non-secret session tokens/configuration. Never store model/API secrets in the app.
- `expo-speech` as a convenience playback path, with server TTS as a later option if iOS voices are insufficient.

## Delivery phases

1. **Mobile chat shell:** separate app, typed composer, chat bubbles, model status, and typed-question API flow.
2. **Reviewable voice input:** permissions, recording, transcript review/editing, cancel/stop states, and approval semantics.
3. **Streaming and recovery:** reply deltas, cancellation, status recovery, reset confirmation, and background/foreground handling.
4. **Attachments:** multipart API, picker UI, validation, progress, extraction, and attachment-aware replies.
5. **TestFlight polish:** accessibility labels, Dynamic Type, VoiceOver checks, offline draft retention, tests, and signing.

## Acceptance criteria for the first useful build

- A person can type a question, edit it, and explicitly send it.
- A person can record a question, edit the transcript, and explicitly send it.
- The reply appears incrementally and remains readable if speech fails.
- Stop/cancel does not submit or regenerate a question.
- A dropped connection allows deliberate status recovery without duplicate submission.
- A file can be selected, reviewed, removed, and sent with a question.
- The app never loses an unsent draft because of a transient API or speech error.
- The existing web app continues to build and test unchanged.

## Decisions needed before implementation

1. Do attachments need only text extraction, or should images be interpreted by the model too?
2. Should conversations persist across app launches, or only for the current session initially?
3. Is the iOS app connecting to a development laptop on the same network, or to a deployed HTTPS API?
4. Should reply speech be optional client-side only, or should the backend eventually provide consistent audio?
