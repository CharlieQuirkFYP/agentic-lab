import { useState } from "react";
import { useOutletContext } from "react-router-dom";
import {
  AlertCircle,
  AudioLines,
  Check,
  CheckCircle2,
  Circle,
  Mic,
  RotateCcw,
  Send,
  ShieldCheck,
  Square,
  Volume2,
  X,
} from "lucide-react";

import type { ConsoleContext } from "@/components/ConsoleLayout";
import { Button } from "@/components/ui/button";
import { API_BASE } from "@/lib/voice/api";
import type { ModelStatus, Turn } from "@/lib/voice/turn";
import { isTerminal } from "@/lib/voice/turn";
import {
  MAX_RECORDING_SECONDS,
  MAX_TEXT_LENGTH,
  MAX_WAV_BYTES,
  validateText,
} from "@/lib/voice/wav";

const fieldClass =
  "w-full rounded-lg border bg-background px-3 py-2 text-sm leading-6 outline-none focus-visible:border-slate-400 focus-visible:ring-2 focus-visible:ring-slate-200 disabled:cursor-not-allowed disabled:bg-muted/50 disabled:text-muted-foreground";
const statusLabels = {
  transcribing: "Transcribing audio",
  awaiting_review: "Awaiting explicit approval",
  generating: "Generating reply",
  completed: "Reply completed",
  failed: "Turn failed",
  cancelled: "Turn cancelled",
};

function ModelBadge({ label, model }: { label: string; model?: ModelStatus }) {
  return (
    <div className="flex min-w-0 items-center gap-2 rounded-lg border bg-white px-3 py-2 text-xs">
      <Circle
        aria-hidden="true"
        className={`size-2 shrink-0 fill-current ${!model ? "text-slate-300" : model.ready ? "text-emerald-600" : "text-amber-600"}`}
      />
      <span className="shrink-0 font-medium">{label}</span>
      <span className="truncate text-muted-foreground" title={model?.name}>
        {model?.name ?? "Unavailable"}
      </span>
      <span className="ml-auto shrink-0 text-muted-foreground">
        {model ? (model.ready ? "Ready" : "Not ready") : "Unknown"}
      </span>
    </div>
  );
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm leading-6 text-amber-950"
    >
      <AlertCircle className="mt-1 size-4 shrink-0" aria-hidden="true" />
      <div>{children}</div>
    </div>
  );
}

function HistoryPair({ turn }: { turn: Turn }) {
  return (
    <article className="space-y-3 border-t pt-4 text-sm leading-6">
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">
          Approved question
        </p>
        <p className="whitespace-pre-wrap break-words">{turn.approved_text}</p>
      </div>
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">
          Completed reply
        </p>
        <p className="whitespace-pre-wrap break-words">{turn.reply}</p>
      </div>
    </article>
  );
}

export default function HomePage() {
  const { voice, microphone, speech } = useOutletContext<ConsoleContext>();
  const [textInput, setTextInput] = useState("");
  const [confirmReset, setConfirmReset] = useState(false);
  const { turn } = voice;
  const micIdle = microphone.phase === "idle";
  const reviewable = turn?.status === "awaiting_review";
  const frozen =
    turn?.approved_text !== null && turn?.approved_text !== undefined;
  const canStart = voice.canStart && micIdle;
  const canEdit =
    reviewable &&
    !voice.command &&
    !voice.recovering &&
    !voice.awaitingApproval;
  const status = !micIdle
    ? microphone.phase === "permission"
      ? "Waiting for microphone permission"
      : microphone.phase === "stopping"
        ? "Finishing WAV recording"
        : "Recording a complete turn"
    : voice.starting
      ? "Opening turn request"
      : turn
        ? statusLabels[turn.status]
        : "Ready for a new question";

  return (
    <main className="mx-auto max-w-7xl space-y-6 px-5 py-8 sm:px-8">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">
            Development workspace
          </p>
          <h1 className="mt-2 text-2xl font-semibold tracking-tight">
            Voice console
          </h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-muted-foreground">
            Record a question, review the wording, then explicitly send it for a
            reply.
          </p>
        </div>
        <Button
          variant="outline"
          disabled={!!voice.command || voice.recovering}
          onClick={() => setConfirmReset(true)}
        >
          <RotateCcw aria-hidden="true" /> Reset web context
        </Button>
      </div>

      {confirmReset && (
        <section
          role="alertdialog"
          aria-labelledby="reset-title"
          aria-describedby="reset-description"
          className="flex flex-wrap items-center justify-between gap-4 rounded-xl border bg-white p-4"
        >
          <div>
            <h2 id="reset-title" className="text-sm font-semibold">
              Clear the shared web conversation?
            </h2>
            <p
              id="reset-description"
              className="mt-1 text-sm text-muted-foreground"
            >
              This cancels active work and clears server web history, including
              what the TUI can inspect. Independent TUI tests are unaffected.
            </p>
          </div>
          <div className="flex gap-2">
            <Button variant="outline" onClick={() => setConfirmReset(false)}>
              Keep context
            </Button>
            <Button
              variant="destructive"
              disabled={!!voice.command || voice.recovering}
              onClick={() => {
                microphone.discard();
                speech.stop();
                setTextInput("");
                setConfirmReset(false);
                void voice.reset();
              }}
            >
              Confirm reset
            </Button>
          </div>
        </section>
      )}

      <div className="grid gap-3 sm:grid-cols-2">
        <ModelBadge label="Transcription" model={voice.readiness?.stt} />
        <ModelBadge label="Reply model" model={voice.readiness?.reply} />
      </div>
      {voice.readinessError && (
        <Notice>
          Initial readiness could not be read: {voice.readinessError} Requests
          still report their own availability. No model is downloaded or
          selected by this page.
        </Notice>
      )}
      {voice.error && <Notice>{voice.error}</Notice>}
      {microphone.error && <Notice>{microphone.error}</Notice>}

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(280px,340px)]">
        <div className="min-w-0 space-y-6">
          <section
            className="rounded-xl border bg-card p-5 sm:p-6"
            aria-labelledby="input-title"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <h2 id="input-title" className="text-base font-semibold">
                1. Ask a question
              </h2>
              <span className="text-xs text-muted-foreground">
                Mono · WAV PCM16 · complete turn
              </span>
            </div>
            <div className="mt-4 flex flex-wrap items-center gap-3">
              {micIdle ? (
                <Button
                  size="lg"
                  disabled={!canStart}
                  onClick={() => {
                    speech.stop();
                    void microphone.start();
                  }}
                >
                  <Mic aria-hidden="true" /> Start microphone
                </Button>
              ) : microphone.phase === "recording" ? (
                <Button
                  size="lg"
                  onClick={() => {
                    void microphone.finish();
                  }}
                >
                  <Square aria-hidden="true" /> Stop & transcribe
                </Button>
              ) : (
                <Button size="lg" disabled>
                  {microphone.phase === "permission"
                    ? "Requesting access…"
                    : "Encoding WAV…"}
                </Button>
              )}
              {!micIdle && (
                <Button variant="outline" onClick={microphone.discard}>
                  <X aria-hidden="true" /> Discard recording
                </Button>
              )}
              <span className="text-xs tabular-nums text-muted-foreground">
                {!micIdle ? `${microphone.seconds}s / ` : "Up to "}
                {MAX_RECORDING_SECONDS}s · {MAX_WAV_BYTES / 1024 / 1024} MiB
                limit
              </span>
            </div>
            <p className="mt-3 text-xs leading-5 text-muted-foreground">
              Stopping sends the whole clip for transcription, not for
              answering. Playback stops before recording. Localhost or HTTPS is
              required for microphone access.
            </p>
            <form
              className="mt-5 space-y-3 border-t pt-5"
              onSubmit={(event) => {
                event.preventDefault();
                if (!canStart || validateText(textInput)) return;
                speech.stop();
                void voice.start(textInput);
                setTextInput("");
              }}
            >
              <label htmlFor="text-question" className="text-sm font-medium">
                Or start with text
              </label>
              <textarea
                id="text-question"
                className={fieldClass}
                rows={3}
                value={textInput}
                maxLength={MAX_TEXT_LENGTH}
                disabled={!canStart}
                onChange={(event) => setTextInput(event.target.value)}
                placeholder="Describe the incident or ask a follow-up question…"
                aria-describedby="text-limit"
              />
              <div className="flex items-center justify-between gap-3">
                <p id="text-limit" className="text-xs text-muted-foreground">
                  {textInput.length} / {MAX_TEXT_LENGTH} characters · review
                  still required
                </p>
                <Button
                  type="submit"
                  variant="outline"
                  disabled={!canStart || !!validateText(textInput)}
                >
                  Start text turn
                </Button>
              </div>
            </form>
          </section>

          <section
            className="rounded-xl border bg-card p-5 sm:p-6"
            aria-labelledby="review-title"
          >
            <div className="flex items-center justify-between gap-3">
              <h2 id="review-title" className="text-base font-semibold">
                2. Review & approve
              </h2>
              {frozen && (
                <span className="flex items-center gap-1 text-xs text-emerald-700">
                  <Check aria-hidden="true" className="size-3.5" /> Accepted
                  wording
                </span>
              )}
            </div>
            <label
              htmlFor="review-question"
              className="mt-4 block text-sm text-muted-foreground"
            >
              {frozen
                ? "Question accepted by the server (web or TUI)"
                : "Edit the transcript before submitting"}
            </label>
            <textarea
              id="review-question"
              className={`${fieldClass} mt-2`}
              rows={5}
              value={voice.editor}
              maxLength={MAX_TEXT_LENGTH}
              disabled={!canEdit}
              onChange={(event) => voice.setEditor(event.target.value)}
              placeholder="Your transcript will appear here. No reply is generated until you or the TUI approve it."
              aria-describedby="review-help"
            />
            <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
              <p
                id="review-help"
                className="max-w-lg text-xs leading-5 text-muted-foreground"
              >
                {frozen
                  ? "Approval freezes this wording. Corrections require a new turn."
                  : "TUI approval updates this editor from the current request stream. Local edits are not shared until submitted."}
              </p>
              <Button
                disabled={!canEdit || !!validateText(voice.editor)}
                onClick={() => {
                  void voice.submit();
                }}
              >
                <Send aria-hidden="true" />
                {voice.command === "submit" || voice.awaitingApproval
                  ? "Awaiting acceptance…"
                  : "Submit approved question"}
              </Button>
            </div>
            {turn?.transcript &&
              turn.transcript !== turn.approved_text &&
              frozen && (
                <details className="mt-4 text-xs text-muted-foreground">
                  <summary className="cursor-pointer">
                    Original transcription
                  </summary>
                  <p className="mt-2 whitespace-pre-wrap break-words leading-5">
                    {turn.transcript}
                  </p>
                </details>
              )}
          </section>

          <section
            className="rounded-xl border bg-card p-5 sm:p-6"
            aria-labelledby="reply-title"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <h2 id="reply-title" className="text-base font-semibold">
                3. Reply
              </h2>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  disabled={
                    turn?.status !== "completed" ||
                    !turn.reply.trim() ||
                    !micIdle ||
                    !!voice.command
                  }
                  onClick={() => speech.speak(turn!.reply)}
                >
                  <Volume2 aria-hidden="true" /> Replay
                </Button>
                <Button
                  variant="outline"
                  disabled={!speech.speaking}
                  onClick={speech.stop}
                >
                  <Square aria-hidden="true" /> Stop voice
                </Button>
              </div>
            </div>
            <div
              className="mt-4 min-h-28 rounded-lg bg-slate-50 p-4 text-sm leading-7"
              aria-busy={turn?.status === "generating"}
            >
              {turn?.reply ? (
                <p className="whitespace-pre-wrap break-words">{turn.reply}</p>
              ) : (
                <p className="text-muted-foreground">
                  {turn?.status === "generating"
                    ? "Waiting for the first reply text…"
                    : "The answer will appear here as the model produces text."}
                </p>
              )}
            </div>
            {turn &&
              ["failed", "cancelled"].includes(turn.status) &&
              turn.reply && (
                <p className="mt-2 text-xs text-amber-800">
                  Incomplete reply — not committed as successful history and not
                  eligible for playback.
                </p>
              )}
            <p className="mt-3 text-xs leading-5 text-muted-foreground">
              Speech starts only after a live, completed reply. Restored
              status/results never auto-play. Replay speaks existing text; it
              does not call the model.
            </p>
          </section>

          {voice.history.length > 0 && (
            <section
              className="space-y-4 rounded-xl border bg-card p-5 sm:p-6"
              aria-labelledby="history-title"
            >
              <h2 id="history-title" className="text-base font-semibold">
                Completed turns in this browser view
              </h2>
              <p className="text-xs text-muted-foreground">
                Up to 20 completed turns kept in memory, not browser storage.
                This is not saved incident-report history.
              </p>
              {voice.history.map((item) => (
                <HistoryPair key={item.turn_id} turn={item} />
              ))}
            </section>
          )}
        </div>

        <aside className="space-y-5">
          <section
            className="rounded-xl border bg-card p-5"
            aria-labelledby="status-title"
          >
            <h2 id="status-title" className="text-sm font-semibold">
              Turn status
            </h2>
            <div
              role="status"
              aria-live="polite"
              className="mt-3 flex items-start gap-2 text-sm leading-6"
            >
              {turn?.status === "completed" ? (
                <CheckCircle2
                  className="mt-1 size-4 shrink-0 text-emerald-700"
                  aria-hidden="true"
                />
              ) : (
                <AudioLines
                  className="mt-1 size-4 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
              )}
              <span>{status}</span>
            </div>
            <p className="mt-2 text-xs leading-5 text-muted-foreground">
              {voice.recovering
                ? "Reading turn status; not resubmitting."
                : voice.connected
                  ? "Current request stream connected."
                  : voice.transportLost
                    ? "Live stream disconnected. Read status for updates; recovery does not resume streaming."
                    : "No live request stream."}
            </p>
            {turn && (
              <p className="mt-3 break-all font-mono text-[11px] text-muted-foreground">
                Turn {turn.turn_id}
              </p>
            )}
            {turn?.error && (
              <p
                role="alert"
                className="mt-3 break-words text-sm text-destructive"
              >
                {turn.error.code}: {turn.error.message}
              </p>
            )}
            <div className="mt-4 flex flex-col gap-2">
              <Button
                variant="outline"
                disabled={
                  !turn ||
                  isTerminal(turn) ||
                  !!voice.command ||
                  voice.recovering
                }
                onClick={() => {
                  speech.stop();
                  void voice.cancel();
                }}
              >
                <Square aria-hidden="true" />
                {turn?.status === "generating"
                  ? "Stop generation"
                  : "Cancel turn"}
              </Button>
              <Button
                variant="outline"
                disabled={!turn || !!voice.command || voice.recovering}
                onClick={() => {
                  void voice.recover();
                }}
              >
                <RotateCcw aria-hidden="true" />
                {voice.recovering ? "Reading status…" : "Read turn status"}
              </Button>
            </div>
            {voice.awaitingApproval && (
              <p className="mt-3 text-xs leading-5 text-muted-foreground">
                Submission acknowledged; waiting for authoritative accepted
                wording. Read status if the stream stalls.
              </p>
            )}
          </section>

          <section
            className="space-y-4 rounded-xl border bg-card p-5"
            aria-labelledby="speech-title"
          >
            <h2
              id="speech-title"
              className="flex items-center gap-2 text-sm font-semibold"
            >
              <Volume2 aria-hidden="true" className="size-4" /> Client speech
              playback
            </h2>
            <label className="flex items-start gap-2 text-sm">
              <input
                className="mt-1 accent-slate-800"
                type="checkbox"
                checked={speech.autoSpeak}
                onChange={(event) => {
                  speech.setAutoSpeak(event.target.checked);
                  if (!event.target.checked) speech.stop();
                }}
              />
              <span>Speak live completed replies</span>
            </label>
            <div>
              <label
                htmlFor="local-voice"
                className="mb-2 block text-xs text-muted-foreground"
              >
                Browser-reported local voice
              </label>
              <select
                id="local-voice"
                className={fieldClass}
                value={speech.voiceURI}
                disabled={!speech.voices.length}
                onChange={(event) => {
                  speech.stop();
                  speech.setVoiceURI(event.target.value);
                }}
              >
                <option value="">
                  {speech.voices.length
                    ? "Default local voice"
                    : "No local voices available"}
                </option>
                {speech.voices.map((item) => (
                  <option key={item.voiceURI} value={item.voiceURI}>
                    {item.name} ({item.lang})
                  </option>
                ))}
              </select>
            </div>
            <p className="text-xs leading-5 text-muted-foreground">
              Only voices marked <code>localService</code> by your browser are
              used. Remote voices are never a fallback. This is an OS/browser
              assertion, not an independently verified offline guarantee.
            </p>
            {!speech.supported && (
              <p className="text-xs leading-5 text-amber-800">
                This browser has no speech-synthesis API. Replies remain
                readable.
              </p>
            )}
            {speech.supported && !speech.voices.length && (
              <p className="text-xs leading-5 text-amber-800">
                Install a local system voice for playback. Voice discovery may
                finish after this page opens.
              </p>
            )}
            {speech.error && <Notice>{speech.error}</Notice>}
          </section>

          <section className="rounded-xl border bg-card p-5 text-xs leading-5 text-muted-foreground">
            <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold text-foreground">
              <ShieldCheck className="size-4" aria-hidden="true" /> Data &
              boundaries
            </h2>
            <p>
              Audio and text go to the configured Go API. On-device inference
              depends on where that API and Pheme actually run; using a phone
              browser does not make remote inference local.
            </p>
            <p className="mt-3 break-all">
              Public API base: <code>{API_BASE}</code>
            </p>
            <p className="mt-3">
              No questions, recordings, or answers are written to browser
              storage. Pheme owns server context. TUI edits can approve this web
              turn, but independent TUI tests never appear in its stream.
            </p>
            <p className="mt-3">
              Readiness badges are an initial snapshot, not a model-management
              interface. Availability errors are shown without downloading
              models.
            </p>
            <p className="mt-3">
              Power, energy, resource usage, and benchmark metrics are
              unavailable here, not zero.
            </p>
          </section>
        </aside>
      </div>
    </main>
  );
}
