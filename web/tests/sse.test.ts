import assert from 'node:assert/strict'
import test from 'node:test'

import { consumeSSE, SSEParser } from '../src/lib/voice/sse.ts'
import type { SSEEvent } from '../src/lib/voice/sse.ts'

const wire = ': heartbeat\r\nevent: transcript.ready\r\ndata: {"turn_id":"t1",\r\ndata: "text":"你好 👋 café"}\r\n\r\nevent: reply.delta\ndata: {"turn_id":"t1","text":"答"}\n\n'
const expected = [
  { event: 'transcript.ready', data: '{"turn_id":"t1",\n"text":"你好 👋 café"}' },
  { event: 'reply.delta', data: '{"turn_id":"t1","text":"答"}' },
]

function stream(chunks: Uint8Array[]) {
  return new ReadableStream<Uint8Array>({ start(controller) {
    for (const chunk of chunks) controller.enqueue(chunk)
    controller.close()
  } })
}

test('SSE handles every UTF-8 byte boundary, CRLF boundary, and multiline frame', async () => {
  const encoded = new TextEncoder().encode(wire)
  for (let boundary = 0; boundary <= encoded.length; boundary++) {
    const events: SSEEvent[] = []
    await consumeSSE(stream([encoded.slice(0, boundary), encoded.slice(boundary)]), (event) => { events.push(event) })
    assert.deepEqual(events, expected, `byte boundary ${boundary}`)
  }
})

test('SSE decodes one byte at a time, including emoji and a UTF-8 BOM', async () => {
  const encoded = new TextEncoder().encode(`\uFEFF${wire}`)
  const events: SSEEvent[] = []
  await consumeSSE(stream(Array.from(encoded, (byte) => new Uint8Array([byte]))), (event) => { events.push(event) })
  assert.deepEqual(events, expected)
})

test('SSE ignores comments and unknown fields; empty data and CR-only frames work', () => {
  const events: SSEEvent[] = []
  const parser = new SSEParser((event) => events.push(event))
  parser.feed(': hi\rid: ignored\rretry: 5\rdata:\r\r')
  parser.finish()
  assert.deepEqual(events, [{ event: 'message', data: '' }])
})

test('unterminated frames are not silently treated as completion', async () => {
  const events: SSEEvent[] = []
  const terminal = await consumeSSE(stream([new TextEncoder().encode('event: reply.completed\ndata: {"text":"partial"}\n')]), (event) => { events.push(event); return true })
  assert.equal(terminal, false)
  assert.deepEqual(events, [])
})

test('terminal event stops reading an open stream and ignores later frames', async () => {
  let cancelled = false
  const events: SSEEvent[] = []
  const source = new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new TextEncoder().encode('event: turn.cancelled\ndata: {}\n\nevent: reply.delta\ndata: late\n\n')) },
    cancel() { cancelled = true },
  })
  assert.equal(await consumeSSE(source, (event) => { events.push(event); return true }), true)
  assert.equal(cancelled, true)
  assert.deepEqual(events, [{ event: 'turn.cancelled', data: '{}' }])
})

test('SSE bounds oversized lines and rejects malformed UTF-8', async () => {
  const parser = new SSEParser(() => {})
  assert.throws(() => parser.feed('x'.repeat(1_048_577)), /size limit/)
  await assert.rejects(consumeSSE(stream([new Uint8Array([0xff])]), () => {}), /encoded data/)
})
