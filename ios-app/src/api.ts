import type { Attachment, Readiness, Turn } from "./types";

const API_BASE = (process.env.EXPO_PUBLIC_API_BASE_URL ?? "http://127.0.0.1:8080/api/v1").replace(/\/$/, "");

function requestId() {
  return `ios-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

type Event = { event: string; data: Record<string, unknown> };

async function checked(response: Response) {
  if (response.ok) return response;
  let message = `API error (${response.status})`;
  try {
    const body = await response.json();
    message = body?.error?.message ?? message;
  } catch { /* non-JSON proxy error */ }
  throw new Error(message);
}

function parseEventBlock(block: string): Event | null {
  const event = block.match(/^event:\s*(.+)$/m)?.[1];
  const data = block.match(/^data:\s*(.+)$/m)?.[1];
  if (!event || !data) return null;
  return { event, data: JSON.parse(data) as Record<string, unknown> };
}

async function readSse(response: Response, onEvent: (event: Event) => void) {
  if (!response.body) throw new Error("The API did not return a readable stream.");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const next = await reader.read();
    buffer += decoder.decode(next.value, { stream: !next.done });
    const blocks = buffer.split(/\r?\n\r?\n/);
    buffer = blocks.pop() ?? "";
    for (const block of blocks) {
      const event = parseEventBlock(block);
      if (event) onEvent(event);
    }
    if (next.done) break;
  }
}

export async function getReadiness(): Promise<Readiness> {
  const response = await checked(await fetch(`${API_BASE}/voice/inspect`));
  return response.json() as Promise<Readiness>;
}

export async function startTurn(input: string | Uint8Array, onEvent: (event: Event) => void, signal: AbortSignal) {
  const response = await checked(await fetch(`${API_BASE}/voice/turns`, {
    method: "POST",
    signal,
    headers: {Accept: "text/event-stream", "Content-Type": typeof input === "string" ? "application/json" : "audio/wav", "Idempotency-Key": requestId()},
    body: (typeof input === "string" ? JSON.stringify({text: input}) : input) as unknown as BodyInit,
  }));
  await readSse(response, onEvent);
}

export async function submitTurn(id: string, text: string) {
  await checked(await fetch(`${API_BASE}/voice/turns/${encodeURIComponent(id)}/submit`, {
    method: "POST", headers: {"Content-Type": "application/json"}, body: JSON.stringify({text}),
  }));
}

export async function cancelTurn(id: string) {
  await checked(await fetch(`${API_BASE}/voice/turns/${encodeURIComponent(id)}/cancel`, {method: "POST"}));
}

export async function getTurn(id: string): Promise<Turn> {
  const response = await checked(await fetch(`${API_BASE}/voice/turns/${encodeURIComponent(id)}`));
  const value = await response.json();
  return {id: value.turn_id, status: value.status, transcript: value.transcript ?? "", approvedText: value.approved_text, reply: value.reply ?? "", error: value.error?.message, attachments: []};
}

export async function uploadAttachments(_attachments: Attachment[]) {
  throw new Error("Attachment upload needs the multipart backend contract; files are kept in the draft for now.");
}
