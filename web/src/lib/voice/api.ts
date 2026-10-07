import { consumeSSE } from "./sse.ts";
import type { SSEEvent } from "./sse.ts";
import { parseReadiness, parseTurn } from "./turn.ts";
import type { Turn } from "./turn.ts";

export const API_BASE = (
  import.meta.env?.VITE_API_BASE_URL || "/api/v1"
).replace(/\/$/, "");
const COMMAND_TIMEOUT_MS = 15_000;

export class VoiceAPIError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(message: string, code: string, status: number) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

async function checked(response: Response): Promise<Response> {
  if (response.ok) return response;
  let message = `Voice API returned HTTP ${response.status}.`;
  let code = "http_error";
  try {
    const body = await response.json();
    if (typeof body.error?.message === "string") message = body.error.message;
    if (typeof body.error?.code === "string") code = body.error.code;
  } catch {
    /* A proxy may return a non-JSON error. */
  }
  throw new VoiceAPIError(message, code, response.status);
}

async function request(path: string, init: RequestInit = {}) {
  const signal = init.signal
    ? AbortSignal.any([init.signal, AbortSignal.timeout(COMMAND_TIMEOUT_MS)])
    : AbortSignal.timeout(COMMAND_TIMEOUT_MS);
  return checked(await fetch(`${API_BASE}/voice${path}`, { ...init, signal }));
}

export async function getReadiness(signal: AbortSignal) {
  return parseReadiness(await (await request("/inspect", { signal })).json());
}
export async function getTurn(id: string, signal?: AbortSignal): Promise<Turn> {
  const turn = parseTurn(
    await (
      await request(`/turns/${encodeURIComponent(id)}`, { signal })
    ).json(),
  );
  if (turn.turn_id !== id)
    throw new Error("Recovery returned a different voice turn.");
  return turn;
}
export async function submitTurn(id: string, text: string) {
  await request(`/turns/${encodeURIComponent(id)}/submit`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
}
export async function cancelTurn(id: string) {
  await request(`/turns/${encodeURIComponent(id)}/cancel`, { method: "POST" });
}
export async function resetVoice() {
  await request("/reset", { method: "POST" });
}
export async function startTurn(
  input: string | Blob,
  signal: AbortSignal,
  onEvent: (event: SSEEvent) => boolean | void,
) {
  // The POST response itself stays open across human review and generation.
  const response = await checked(
    await fetch(`${API_BASE}/voice/turns`, {
      method: "POST",
      signal,
      headers: {
        Accept: "text/event-stream",
        "Content-Type":
          typeof input === "string" ? "application/json" : "audio/wav",
        "Idempotency-Key": crypto.randomUUID(),
      },
      body: typeof input === "string" ? JSON.stringify({ text: input }) : input,
    }),
  );
  if (
    !response.headers
      .get("content-type")
      ?.toLowerCase()
      .includes("text/event-stream") ||
    !response.body
  ) {
    throw new Error("Voice API did not return a readable event stream.");
  }
  return consumeSSE(response.body, onEvent);
}

export function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "An unexpected voice operation error occurred.";
}
