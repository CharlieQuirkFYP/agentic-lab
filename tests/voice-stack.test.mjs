import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  chmod,
  copyFile,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { consumeSSE } from "../web/src/lib/voice/sse.ts";
import { applyTurnEvent } from "../web/src/lib/voice/turn.ts";

const root = fileURLToPath(new URL("../", import.meta.url));
const workspace = path.join(root, "pheme-va");
const serverBinary = path.join(workspace, "target/debug/server");
const apiBinary = path.join(workspace, "target/integration-api");

async function freePort() {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1");
  await once(socket, "listening");
  const port = socket.address().port;
  await new Promise((resolve, reject) =>
    socket.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

async function until(predicate, label) {
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await delay(20);
  }
  throw new Error(`Timed out: ${label}`);
}

function start(binary, args, env, cwd) {
  const child = spawn(binary, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let logs = "";
  const append = (bytes) => {
    logs = (logs + bytes.toString()).slice(-65536);
  };
  child.stdout.on("data", append);
  child.stderr.on("data", append);
  child.on("error", append);
  return { child, logs: () => logs };
}

async function stop(process) {
  if (process.child.exitCode !== null || process.child.signalCode !== null)
    return;
  const exited = once(process.child, "exit");
  process.child.kill("SIGTERM");
  const timer = setTimeout(() => process.child.kill("SIGKILL"), 3000);
  try {
    await exited;
  } finally {
    clearTimeout(timer);
  }
}

function stream(response, browserTurn = false) {
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type"), /text\/event-stream/);
  const events = [];
  let turn = null;
  const done = consumeSSE(response.body, (event) => {
    const data = JSON.parse(event.data);
    events.push({ name: event.event, data });
    if (browserTurn) turn = applyTurnEvent(turn, event);
    return [
      "reply.completed",
      "turn.failed",
      "reply.failed",
      "turn.cancelled",
    ].includes(event.event);
  });
  // Attach rejection handling immediately; explicit awaits below still surface failures.
  done.catch(() => {});
  return { events, done, turn: () => turn };
}

test(
  "web-owned loop, TUI inspection/approval, and isolated tests across Go and Pheme",
  { timeout: 45000 },
  async (t) => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "pheme-stack-"));
    const worker = path.join(dir, "worker.sh");
    await copyFile(
      path.join(workspace, "crates/models/reply/tests/fixtures/worker.sh"),
      worker,
    );
    await chmod(worker, 0o700);
    await writeFile(
      path.join(dir, "valid.gguf"),
      "tiny fake model, not weights",
    );
    await writeFile(path.join(dir, "choices.toml"), "");
    const additionalRole =
      "Additional local role: preserve the approved wording.\n";
    const additionalRolePath = path.join(dir, "additional-role.txt");
    await writeFile(additionalRolePath, additionalRole);
    const combinedRole =
      (await readFile(
        path.join(workspace, "roles/incident-reporting.txt"),
        "utf8",
      )) +
      "\n\n" +
      additionalRole;
    const rustPort = await freePort();
    const goPort = await freePort();
    const rustURL = `http://127.0.0.1:${rustPort}`;
    const goURL = `http://127.0.0.1:${goPort}`;
    const pheme = start(
      serverBinary,
      [
        "--bind",
        `127.0.0.1:${rustPort}`,
        "--reply-path",
        path.join(dir, "valid.gguf"),
        "--prompt-file",
        path.join(workspace, "roles/incident-reporting.txt"),
        "--prompt-file",
        additionalRolePath,
        "--startup-choices",
        path.join(dir, "choices.toml"),
        "--max-review-seconds",
        "15",
        "--metrics-enabled",
      ],
      { PHEME_VA_REPLY_WORKER: worker, PHEME_VA_WHISPER_MODEL: undefined },
      workspace,
    );
    const api = start(
      apiBinary,
      [],
      {
        API_ADDR: `127.0.0.1:${goPort}`,
        PHEME_VA_URL: rustURL,
        API_CORS_ORIGINS: "http://localhost:5173",
        GIN_MODE: "release",
      },
      root,
    );
    t.after(async () => {
      await Promise.all([stop(api), stop(pheme)]);
      await rm(dir, { recursive: true, force: true });
    });
    for (const [url, process] of [
      [rustURL, pheme],
      [goURL, api],
    ]) {
      await until(async () => {
        if (process.child.exitCode !== null) throw new Error(process.logs());
        try {
          return (
            await fetch(`${url}/health`, { signal: AbortSignal.timeout(500) })
          ).ok;
        } catch {
          return false;
        }
      }, `${url} ready; build binaries first (see tests/README.md)`);
    }
    const base = `${goURL}/api/v1/voice`;
    const request = (route, text, extra = {}) =>
      fetch(`${base}${route}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Origin: "http://localhost:5173",
          ...extra.headers,
        },
        body: text === undefined ? undefined : JSON.stringify({ text }),
        signal: extra.signal ?? AbortSignal.timeout(10000),
      });
    const inspect = async () => {
      const response = await fetch(`${base}/inspect`, {
        signal: AbortSignal.timeout(2000),
      });
      assert.equal(response.status, 200);
      return response.json();
    };
    await until(async () => !(await inspect()).busy, "inference idle");

    await t.test(
      "untrusted browser origins cannot mutate the development conversation",
      async () => {
        const denied = await request("/reset", undefined, {
          headers: { Origin: "https://untrusted.example" },
        });
        assert.equal(denied.status, 403);
        assert.equal((await denied.json()).error.code, "origin_not_allowed");
        assert.deepEqual((await inspect()).history, []);
      },
    );

    await t.test(
      "TUI test while web reviews does not enter the web stream or history",
      async () => {
        const readiness = await inspect();
        assert.equal(readiness.reply.ready, true);
        assert.equal(readiness.stt.ready, false);
        const web = stream(
          await request("/turns", "uncorrected question", {
            headers: { "Idempotency-Key": "web-one" },
          }),
          true,
        );
        await until(
          () => web.events.some((event) => event.name === "transcript.ready"),
          "review transcript",
        );
        const id = web.turn().turn_id;
        assert.equal(web.turn().status, "awaiting_review");
        assert.equal((await inspect()).current_turn.turn_id, id);
        const isolated = stream(
          await request("/test/reply", "independent TUI question"),
        );
        assert.equal(await isolated.done, true);
        assert.equal(isolated.events.at(-1).name, "reply.completed");
        const afterTest = await inspect();
        assert.deepEqual(afterTest.history, []);
        assert.equal(afterTest.current_turn.transcript, "uncorrected question");
        assert.equal(afterTest.current_turn.reply, "");
        assert.equal(
          web.events.some((event) => event.name === "reply.delta"),
          false,
        );
        await until(async () => !(await inspect()).busy, "test settled");

        const approved = "  No fire — café west entrance.\n你好 ";
        const ack = await request(`/turns/${id}/submit`, approved);
        assert.equal(ack.status, 200);
        assert.equal((await ack.json()).approved_text, approved);
        assert.equal(await web.done, true);
        assert.equal(web.turn().approved_text, approved);
        assert.equal(web.turn().reply, "Hello café 世界 🧯");
        const accepted = web.events.find(
          (event) => event.name === "question.approved",
        );
        assert.equal(accepted.data.text, approved);
        const delta = web.events
          .filter((event) => event.name === "reply.delta")
          .map((event) => event.data.text)
          .join("");
        assert.equal(delta, web.turn().reply);
        await until(async () => !(await inspect()).busy, "web settled");
        const completed = await inspect();
        assert.deepEqual(
          completed.history.map((message) => message.content),
          [approved, web.turn().reply],
        );
        const status = await (await fetch(`${base}/turns/${id}`)).json();
        assert.equal(status.status, "completed");
        assert.equal(status.reply, web.turn().reply);
        assert.equal(
          (await request(`/turns/${id}/submit`, approved)).status,
          200,
        );
        assert.equal(
          (await request(`/turns/${id}/submit`, "different correction")).status,
          409,
        );
      },
    );

    await t.test(
      "web follows successful history; TUI test uses fresh context; weights stay resident",
      async () => {
        const before = await inspect();
        const isolated = stream(
          await request("/test/reply", "another independent question"),
        );
        assert.equal(await isolated.done, true);
        await until(async () => !(await inspect()).busy, "second test settled");
        assert.deepEqual((await inspect()).history, before.history);
        const web = stream(await request("/turns", "follow-up question"), true);
        await until(
          () => web.turn()?.status === "awaiting_review",
          "second web review",
        );
        assert.equal(
          (
            await request(
              `/turns/${web.turn().turn_id}/submit`,
              "follow-up approved",
            )
          ).status,
          200,
        );
        await web.done;
        await until(async () => !(await inspect()).busy, "follow-up settled");
        const commands = (await readFile(`${worker}.log`, "utf8"))
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line));
        assert.equal(
          commands.filter((command) => command.type === "init").length,
          1,
        );
        const generations = commands.filter(
          (command) => command.type === "generate",
        );
        for (const generation of generations) {
          assert.equal(generation.messages[0].role, "system");
          assert.equal(generation.messages[0].content, combinedRole);
        }
        assert.equal(generations[0].messages.length, 2);
        assert.equal(
          generations[0].messages[1].content,
          "independent TUI question",
        );
        assert.equal(
          generations[1].messages[1].content,
          before.history[0].content,
        );
        assert.equal(generations[2].messages.length, 2);
        assert.equal(generations[3].messages.length, 4);
        assert.equal(
          generations[3].messages.at(-1).content,
          "follow-up approved",
        );
      },
    );

    await t.test(
      "native test cancellation reaches the worker without cancelling web state",
      async () => {
        const before = await inspect();
        const controller = new AbortController();
        const isolated = stream(
          await request("/test/reply", "WAIT", { signal: controller.signal }),
        );
        await until(
          () => isolated.events.some((event) => event.name === "reply.delta"),
          "test partial output",
        );
        controller.abort();
        await isolated.done.catch(() => {});
        await until(
          async () => !(await inspect()).busy,
          "native test cancellation",
        );
        const after = await inspect();
        assert.deepEqual(after.history, before.history);
        assert.deepEqual(after.current_turn, before.current_turn);
        const commands = (await readFile(`${worker}.log`, "utf8"))
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line));
        assert.equal(commands.at(-1).type, "cancel");
        assert.equal(after.reply.ready, true);
      },
    );

    await t.test(
      "web cancellation/reset settles work and no HTTP download route exists",
      async () => {
        const web = stream(await request("/turns", "WAIT"), true);
        await until(
          () => web.turn()?.status === "awaiting_review",
          "cancel-turn review",
        );
        const id = web.turn().turn_id;
        assert.equal(
          (await request(`/turns/${id}/submit`, "WAIT")).status,
          200,
        );
        await until(
          () => web.events.some((event) => event.name === "reply.delta"),
          "web partial reply",
        );
        assert.equal((await request(`/turns/${id}/cancel`)).status, 200);
        await web.done;
        assert.equal(web.turn().status, "cancelled");
        assert.equal((await request("/reset")).status, 200);
        const after = await inspect();
        assert.deepEqual(after.history, []);
        assert.equal(after.current_turn, null);
        assert.equal(after.busy, false);
        assert.equal((await fetch(`${base}/turns/${id}`)).status, 404);
        assert.equal(
          (await request("/models/download", "anything")).status,
          404,
        );
      },
    );
  },
);
