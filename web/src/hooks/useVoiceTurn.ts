import { useEffect, useLayoutEffect, useRef, useState } from "react";

import {
  cancelTurn,
  errorMessage,
  getReadiness,
  getTurn,
  resetVoice,
  startTurn,
  submitTurn,
  VoiceAPIError,
} from "@/lib/voice/api";
import { applyTurnEvent, isTerminal } from "@/lib/voice/turn";
import type { Readiness, Turn } from "@/lib/voice/turn";
import { validateText } from "@/lib/voice/wav";

export function useVoiceTurn(onLiveReply: (text: string) => void) {
  const [turn, setTurn] = useState<Turn | null>(null);
  const [editor, setEditor] = useState("");
  const [history, setHistory] = useState<Turn[]>([]);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [readinessError, setReadinessError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [command, setCommand] = useState<"submit" | "cancel" | "reset" | null>(
    null,
  );
  const [connected, setConnected] = useState(false);
  const [recovering, setRecovering] = useState(false);
  const [transportLost, setTransportLost] = useState(false);
  const [awaitingApproval, setAwaitingApproval] = useState(false);
  const streamOpen = useRef(false);
  const version = useRef(0);
  const silenceTurn = useRef(false);
  const current = useRef<Turn | null>(null);
  const epoch = useRef(0);
  const stream = useRef<AbortController | null>(null);
  const startingRef = useRef(false);
  const commandRef = useRef<string | null>(null);
  const recoveryRef = useRef(false);
  const onLiveReplyRef = useRef(onLiveReply);
  useLayoutEffect(() => {
    onLiveReplyRef.current = onLiveReply;
  }, [onLiveReply]);

  function publish(next: Turn | null) {
    version.current++;
    current.current = next;
    setTurn(next);
  }
  function mutation(value: typeof command) {
    commandRef.current = value;
    setCommand(value);
  }

  useEffect(() => {
    const controller = new AbortController();
    void getReadiness(controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) setReadiness(value);
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setReadinessError(errorMessage(cause));
      });
    const stopRequest = () => {
      epoch.current++;
      stream.current?.abort();
    };
    const leavePage = () => {
      const unfinished =
        startingRef.current ||
        (current.current && !isTerminal(current.current));
      stopRequest();
      streamOpen.current = false;
      startingRef.current = false;
      commandRef.current = null;
      setStarting(false);
      setCommand(null);
      setConnected(false);
      if (unfinished) setTransportLost(true);
    };
    window.addEventListener("pagehide", leavePage);
    return () => {
      controller.abort();
      window.removeEventListener("pagehide", leavePage);
      stopRequest();
    };
  }, []);

  async function recover(id = current.current?.turn_id, token = epoch.current) {
    if (!id || recoveryRef.current) return;
    recoveryRef.current = true;
    setRecovering(true);
    const requestedVersion = version.current;
    try {
      const recovered = await getTurn(id);
      if (token !== epoch.current) return;
      // Do not replace newer live deltas/terminal events with an older GET snapshot.
      if (version.current !== requestedVersion) return;
      const previous = current.current;
      publish(recovered);
      setAwaitingApproval(false);
      if (recovered.approved_text !== null) setEditor(recovered.approved_text);
      else if (!previous || previous.status === "transcribing")
        setEditor(recovered.transcript);
      // No inference retry and no playback, even if recovery finds a completed reply.
      setError(null);
      if (isTerminal(recovered)) setTransportLost(false);
    } catch (cause) {
      if (token === epoch.current)
        setError(
          `Status recovery failed: ${errorMessage(cause)} No question was resubmitted. If the turn is no longer retained, reset explicitly.`,
        );
    } finally {
      recoveryRef.current = false;
      setRecovering(false);
    }
  }

  async function start(input: string | Blob) {
    if (
      transportLost ||
      startingRef.current ||
      commandRef.current ||
      recoveryRef.current ||
      (current.current && !isTerminal(current.current))
    )
      return;
    if (typeof input === "string") {
      const invalid = validateText(input);
      if (invalid) {
        setError(invalid);
        return;
      }
    }
    const previous = current.current;
    if (previous?.status === "completed")
      setHistory((items) => [...items, previous].slice(-20));
    stream.current?.abort();
    const controller = new AbortController();
    stream.current = controller;
    const token = ++epoch.current;
    silenceTurn.current = false;
    streamOpen.current = false;
    setAwaitingApproval(false);
    startingRef.current = true;
    setStarting(true);
    setTransportLost(false);
    setConnected(false);
    setRecovering(false);
    setError(null);
    setEditor("");
    publish(null);
    let liveCompletion = false;
    let handshakeTimedOut = false;
    const handshakeTimeout = setTimeout(() => {
      handshakeTimedOut = true;
      controller.abort();
    }, 15_000);
    try {
      const terminal = await startTurn(input, controller.signal, (event) => {
        if (token !== epoch.current || controller.signal.aborted) return true;
        const previous = current.current;
        const next = applyTurnEvent(previous, event);
        if (next !== previous && next) {
          publish(next);
          clearTimeout(handshakeTimeout);
          streamOpen.current = true;
          setConnected(true);
          startingRef.current = false;
          setStarting(false);
          if (event.event === "transcript.ready") setEditor(next.transcript);
          if (event.event === "question.approved") {
            setEditor(next.approved_text ?? "");
            setAwaitingApproval(false);
          }
          if (next.status === "completed" && !liveCompletion) {
            liveCompletion = true;
            if (
              !silenceTurn.current &&
              commandRef.current !== "reset" &&
              commandRef.current !== "cancel"
            )
              onLiveReplyRef.current(next.reply);
          }
        }
        return isTerminal(next);
      });
      if (!terminal && token === epoch.current)
        throw new Error("The voice stream ended before the turn completed.");
    } catch (cause) {
      if (
        (controller.signal.aborted && !handshakeTimedOut) ||
        token !== epoch.current
      )
        return;
      streamOpen.current = false;
      setConnected(false);
      setTransportLost(true);
      const id = current.current?.turn_id;
      if (id) {
        setError(
          `${errorMessage(cause)} Reading this turn's status without resubmitting.`,
        );
        await recover(id, token);
      } else if (
        cause instanceof VoiceAPIError &&
        ([400, 401, 403, 404, 405, 409, 413, 415, 422, 429].includes(
          cause.status,
        ) ||
          ["busy", "not_ready", "model_not_ready"].includes(cause.code))
      ) {
        setTransportLost(false);
        setError(
          `${cause.code}: ${cause.message} The request was rejected; nothing was retried or downloaded.`,
        );
      } else {
        const reason = handshakeTimedOut
          ? "No turn ID was received within 15 seconds."
          : errorMessage(cause);
        setError(
          `${reason} The server may have received the request, but no turn ID arrived. Nothing will be retried automatically. Reset the server context before starting again.`,
        );
      }
    } finally {
      clearTimeout(handshakeTimeout);
      if (token === epoch.current) {
        startingRef.current = false;
        setStarting(false);
        streamOpen.current = false;
        setConnected(false);
      }
    }
  }

  async function submit() {
    const pending = current.current;
    if (
      !pending ||
      pending.status !== "awaiting_review" ||
      awaitingApproval ||
      commandRef.current ||
      recoveryRef.current
    )
      return;
    const invalid = validateText(editor);
    if (invalid) {
      setError(invalid);
      return;
    }
    const token = epoch.current;
    mutation("submit");
    setError(null);
    try {
      await submitTurn(pending.turn_id, editor);
      // Accepted text is authoritative only in SSE/status, not our local editor.
      if (token === epoch.current) {
        if (streamOpen.current)
          setAwaitingApproval(current.current?.status === "awaiting_review");
        else await recover(pending.turn_id, token);
      }
    } catch (cause) {
      const message = errorMessage(cause);
      // A conflict/timeout can mean the TUI (or this POST) already won approval.
      if (token === epoch.current) {
        await recover(pending.turn_id, token);
        setError(
          `${message} Status was checked; the submission was not retried.`,
        );
      }
    } finally {
      if (token === epoch.current) mutation(null);
    }
  }

  async function cancel() {
    const pending = current.current;
    if (!pending || isTerminal(pending) || commandRef.current) return;
    const token = epoch.current;
    mutation("cancel");
    silenceTurn.current = true;
    setError(null);
    try {
      await cancelTurn(pending.turn_id);
      if (token !== epoch.current) return;
      stream.current?.abort();
      streamOpen.current = false;
      setConnected(false);
      setTransportLost(true);
      await recover(pending.turn_id, token);
    } catch (cause) {
      if (token === epoch.current)
        setError(
          `Cancellation was not confirmed: ${errorMessage(cause)} Use Read turn status before trying again.`,
        );
    } finally {
      if (token === epoch.current) mutation(null);
    }
  }

  async function reset() {
    if (commandRef.current || recoveryRef.current) return;
    mutation("reset");
    // Invalidate callbacks immediately; a late reply must not repopulate reset state.
    const token = ++epoch.current;
    silenceTurn.current = true;
    streamOpen.current = false;
    setAwaitingApproval(false);
    stream.current?.abort();
    startingRef.current = false;
    setStarting(false);
    setConnected(false);
    setTransportLost(true);
    setError(null);
    try {
      await resetVoice();
      if (token !== epoch.current) return;
      publish(null);
      setHistory([]);
      setEditor("");
      setTransportLost(false);
    } catch (cause) {
      if (token === epoch.current)
        setError(
          `Reset was not confirmed: ${errorMessage(cause)} Existing text is preserved. Read turn status or explicitly retry reset; no new turn was sent.`,
        );
    } finally {
      if (token === epoch.current) mutation(null);
    }
  }

  return {
    turn,
    editor,
    setEditor,
    history,
    readiness,
    readinessError,
    error,
    starting,
    command,
    connected,
    recovering,
    transportLost,
    awaitingApproval,
    canStart:
      !starting &&
      !command &&
      !recovering &&
      !transportLost &&
      (!turn || isTerminal(turn)),
    start,
    submit,
    cancel,
    reset,
    recover: () => recover(),
  };
}
