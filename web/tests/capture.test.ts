import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { runInNewContext } from "node:vm";

interface Worklet {
  port: { onmessage: (message: { data: string }) => void };
  process: (inputs: Float32Array[][]) => boolean;
}

test("audio worklet downmixes PCM, flushes the tail, and stops without monitor playback", async () => {
  const messages: Array<{ samples?: Float32Array; stopped?: boolean }> = [];
  let Processor: (new () => Worklet) | undefined;
  const source = await readFile(
    new URL("../public/pcm-capture.worklet.js", import.meta.url),
    "utf8",
  );
  runInNewContext(source, {
    Float32Array,
    AudioWorkletProcessor: class {
      port = {
        onmessage: undefined,
        postMessage(message: { samples?: Float32Array; stopped?: boolean }) {
          messages.push(message);
        },
      };
    },
    registerProcessor(name: string, processor: new () => Worklet) {
      assert.equal(name, "pcm-capture");
      Processor = processor;
    },
  });
  assert.ok(Processor);
  const worklet = new Processor();
  assert.equal(
    worklet.process([[new Float32Array([1, 0]), new Float32Array([0, -1])]]),
    true,
  );
  assert.equal(messages.length, 0);
  worklet.port.onmessage({ data: "stop" });
  assert.deepEqual(Array.from(messages[0].samples!), [0.5, -0.5]);
  assert.equal(messages[1].stopped, true);
  assert.equal(worklet.process([[new Float32Array([1])]]), false);
  assert.equal(messages.length, 2);
});
