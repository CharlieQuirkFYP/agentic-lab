import assert from "node:assert/strict";
import test from "node:test";

import {
  API_BASE,
  cancelTurn,
  getReadiness,
  getTurn,
  resetVoice,
  startTurn,
  submitTurn,
  VoiceAPIError,
} from "../src/lib/voice/api.ts";
import { applyTurnEvent } from "../src/lib/voice/turn.ts";
import type { Turn } from "../src/lib/voice/turn.ts";
import { encodeWav } from "../src/lib/voice/wav.ts";

function sse(frames: [string, Record<string, unknown>][]) {
  return new Response(
    frames
      .map(
        ([name, data]) => `event: ${name}\ndata: ${JSON.stringify(data)}\n\n`,
      )
      .join(""),
    { headers: { "Content-Type": "text/event-stream" } },
  );
}

test("public text POST consumes TUI acceptance and real incremental frames from its own response", async (t) => {
  let requests = 0;
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    requests++;
    assert.equal(url, "/api/v1/voice/turns");
    assert.equal(init.method, "POST");
    assert.equal(init.headers!["Content-Type"], "application/json");
    assert.equal(JSON.parse(init.body as string).text, "  Exact input 👋  ");
    assert.ok(init.headers!["Idempotency-Key"]);
    return sse([
      ["turn.created", { turn_id: "web" }],
      ["transcript.ready", { turn_id: "web", text: "Original" }],
      ["question.approved", { turn_id: "web", text: "TUI accepted question" }],
      ["reply.started", { turn_id: "web" }],
      ["reply.delta", { turn_id: "web", text: "Incremental " }],
      ["reply.delta", { turn_id: "web", text: "answer" }],
      ["reply.completed", { turn_id: "web", text: "Final answer" }],
    ]);
  });
  let turn: Turn | null = null;
  const partials: string[] = [];
  await startTurn(
    "  Exact input 👋  ",
    new AbortController().signal,
    (frame) => {
      turn = applyTurnEvent(turn, frame);
      if (frame.event === "reply.delta") partials.push(turn!.reply);
      return turn?.status === "completed";
    },
  );
  assert.equal(API_BASE, "/api/v1");
  assert.deepEqual(partials, ["Incremental ", "Incremental answer"]);
  assert.equal(turn!.approved_text, "TUI accepted question");
  assert.equal(turn!.reply, "Final answer");
  assert.equal(requests, 1);
});

test("audio starts as raw audio/wav, never multipart or WebM", async (t) => {
  const clip = new Blob(
    [encodeWav([new Float32Array([0.25, -0.25])], 16_000)],
    { type: "audio/wav" },
  );
  t.mock.method(
    globalThis,
    "fetch",
    async (_url: string, init: RequestInit) => {
      assert.equal(init.headers!["Content-Type"], "audio/wav");
      assert.equal(init.body, clip);
      const bytes = await (init.body as Blob).arrayBuffer();
      assert.equal(new TextDecoder().decode(bytes.slice(0, 4)), "RIFF");
      return sse([
        ["turn.created", { turn_id: "audio" }],
        ["turn.cancelled", { turn_id: "audio" }],
      ]);
    },
  );
  assert.equal(
    await startTurn(
      clip,
      new AbortController().signal,
      (frame) => frame.event === "turn.cancelled",
    ),
    true,
  );
});

test("status recovery uses only GET, validates identity, and never retries a missing turn", async (t) => {
  const calls: string[] = [];
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    calls.push(url);
    assert.equal(init.method, undefined);
    return Response.json({
      turn_id: "a/b",
      status: "completed",
      transcript: "Before",
      approved_text: "Accepted",
      reply: "Recovered",
      error: null,
      timings: {},
    });
  });
  assert.equal((await getTurn("a/b")).reply, "Recovered");
  assert.deepEqual(calls, ["/api/v1/voice/turns/a%2Fb"]);
  t.mock.method(globalThis, "fetch", async () =>
    Response.json(
      {
        error: {
          code: "turn_not_found",
          message: "Turn is no longer retained",
        },
      },
      { status: 404 },
    ),
  );
  await assert.rejects(getTurn("a/b"), /no longer retained/);
  t.mock.method(globalThis, "fetch", async () =>
    Response.json({
      turn_id: "other",
      status: "completed",
      reply: "Wrong identity",
    }),
  );
  await assert.rejects(getTurn("a/b"), /different voice turn/);
});

test("submit/cancel/reset use separate public commands and submit preserves exact editor text", async (t) => {
  const calls: Array<{ url: string; init: RequestInit }> = [];
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    return new Response(null, { status: 204 });
  });
  await submitTurn("web", "  Edited wording  ");
  await cancelTurn("web");
  await resetVoice();
  assert.deepEqual(
    calls.map(({ url }) => url),
    [
      "/api/v1/voice/turns/web/submit",
      "/api/v1/voice/turns/web/cancel",
      "/api/v1/voice/reset",
    ],
  );
  assert.ok(calls.every(({ init }) => init.method === "POST"));
  assert.equal(calls[0].init.body, '{"text":"  Edited wording  "}');
});

test("not-ready errors preserve server code and never download or retry", async (t) => {
  let requests = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    return Response.json(
      { error: { code: "not_ready", message: "Reply model unavailable" } },
      { status: 503 },
    );
  });
  await assert.rejects(
    startTurn("question", new AbortController().signal, () => {}),
    (error: unknown) => {
      assert.ok(error instanceof VoiceAPIError);
      assert.equal(error.status, 503);
      assert.equal(error.code, "not_ready");
      assert.equal(error.message, "Reply model unavailable");
      return true;
    },
  );
  assert.equal(requests, 1);
});

test("initial inspection only reads readiness; non-SSE responses are honest transport errors", async (t) => {
  t.mock.method(globalThis, "fetch", async (url: string) => {
    assert.equal(url, "/api/v1/voice/inspect");
    return Response.json({
      stt: { name: "STT", ready: false },
      reply: { name: "Reply", ready: true },
      history: [],
    });
  });
  assert.equal(
    (await getReadiness(new AbortController().signal)).stt.ready,
    false,
  );
  t.mock.method(globalThis, "fetch", async () =>
    Response.json({ text: "Not a stream" }),
  );
  await assert.rejects(
    startTurn("question", new AbortController().signal, () => {}),
    /readable event stream/,
  );
});
