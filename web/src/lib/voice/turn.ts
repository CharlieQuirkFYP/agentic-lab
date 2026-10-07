import { MAX_REPLY_LENGTH } from "./wav.ts";
import type { SSEEvent } from "./sse.ts";

export type TurnStatus =
  | "transcribing"
  | "awaiting_review"
  | "generating"
  | "completed"
  | "failed"
  | "cancelled";
export interface TurnError {
  code: string;
  message: string;
}
export interface Turn {
  turn_id: string;
  status: TurnStatus;
  transcript: string;
  approved_text: string | null;
  reply: string;
  error: TurnError | null;
  timings: Record<string, number | null>;
}
export interface ModelStatus {
  name: string;
  ready: boolean;
}
export interface Readiness {
  stt: ModelStatus;
  reply: ModelStatus;
}

export function isTerminal(turn: Turn | null): boolean {
  return !!turn && ["completed", "failed", "cancelled"].includes(turn.status);
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid voice response.");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string")
    throw new Error("Invalid voice response text.");
  return value;
}
function turnError(value: unknown): TurnError {
  const error = object(value);
  return { code: text(error.code), message: text(error.message) };
}
function replyText(value: unknown): string {
  const result = text(value);
  if (result.length > MAX_REPLY_LENGTH)
    throw new Error("Reply exceeds the client display limit.");
  return result;
}

export function parseTurn(value: unknown): Turn {
  const data = object(value);
  const status = text(data.status);
  if (
    ![
      "transcribing",
      "awaiting_review",
      "generating",
      "completed",
      "failed",
      "cancelled",
    ].includes(status)
  ) {
    throw new Error("Unknown voice turn status.");
  }
  const timings: Turn["timings"] = {};
  if (data.timings && typeof data.timings === "object") {
    for (const [key, value] of Object.entries(data.timings)) {
      if (
        value === null ||
        (typeof value === "number" && Number.isFinite(value))
      )
        timings[key] = value;
    }
  }
  return {
    turn_id: text(data.turn_id),
    status: status as TurnStatus,
    transcript: text(data.transcript ?? ""),
    approved_text: data.approved_text == null ? null : text(data.approved_text),
    reply: replyText(data.reply ?? ""),
    error: data.error == null ? null : turnError(data.error),
    timings,
  };
}

export function parseReadiness(value: unknown): Readiness {
  const data = object(value);
  const model = (value: unknown): ModelStatus => {
    const data = object(value);
    if (typeof data.ready !== "boolean")
      throw new Error("Invalid model readiness response.");
    return {
      name: data.name == null ? "Not configured" : text(data.name),
      ready: data.ready,
    };
  };
  // Inspection is used once for badges only, never as a conversation feed.
  return { stt: model(data.stt), reply: model(data.reply) };
}

export const TURN_EVENTS = new Set([
  "turn.created",
  "transcript.ready",
  "question.approved",
  "reply.started",
  "reply.delta",
  "reply.completed",
  "turn.failed",
  "reply.failed",
  "turn.cancelled",
]);

export function applyTurnEvent(
  current: Turn | null,
  event: SSEEvent,
): Turn | null {
  if (!TURN_EVENTS.has(event.event)) return current;
  const data = object(JSON.parse(event.data));
  const id = text(data.turn_id);
  if (current && (current.turn_id !== id || isTerminal(current)))
    return current;
  if (!current) {
    if (event.event !== "turn.created")
      throw new Error("Voice stream did not identify its turn.");
    return {
      turn_id: id,
      status: "transcribing",
      transcript: "",
      approved_text: null,
      reply: "",
      error: null,
      timings: {},
    };
  }
  switch (event.event) {
    case "transcript.ready":
      if (current.status !== "transcribing") return current;
      return {
        ...current,
        status: "awaiting_review",
        transcript: text(data.text),
      };
    case "question.approved": {
      const approved = text(data.text);
      if (current.approved_text !== null) {
        if (current.approved_text !== approved)
          throw new Error("The server changed an already approved question.");
        return current;
      }
      return {
        ...current,
        status: "generating",
        approved_text: approved,
        reply: "",
      };
    }
    case "reply.started":
      if (current.approved_text === null)
        throw new Error("Reply started without an approved question.");
      return { ...current, status: "generating" };
    case "reply.delta":
      if (current.status !== "generating" || current.approved_text === null)
        throw new Error("Unexpected reply text before approval.");
      return { ...current, reply: replyText(current.reply + text(data.text)) };
    case "reply.completed":
      if (current.approved_text === null)
        throw new Error("Reply completed without an approved question.");
      return { ...current, status: "completed", reply: replyText(data.text) };
    case "turn.failed":
    case "reply.failed":
      return { ...current, status: "failed", error: turnError(data.error) };
    case "turn.cancelled":
      return { ...current, status: "cancelled" };
    default:
      return current;
  }
}
