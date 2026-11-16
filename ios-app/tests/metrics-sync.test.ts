import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createMetricsOutbox, normalizeMetricsURL, type MetricsConnection, type MetricsUpload, type MetricBatch } from '../src/metricsOutbox.ts';
import { createMetricsSync } from '../src/metricsSync.ts';
import type { Observation } from '../modules/pheme-native/src/PhemeNative.types.ts';
import { replyMetrics, transcriptionMetadata } from '../src/phoneMetricEvents.ts';

const device = { installation_id: 'test-install', hardware: 'iPhone-test', os_version: '27', app_version: '0.1', build: '2' };
function batch(run = 'run-1'): MetricBatch {
  return { schema_version: 1, run_id: run, sequence_number: 0, timestamp_ms: 1000,
    events: [{ schema_version: 1, run_id: run, sequence: 0, timestamp_ms: 1000,
      name: 'reasoning_duration_ms', value: 123, unit: 'milliseconds', scope: 'run', source: 'native.ios.host' }] };
}
function upload(run = 'run-1'): MetricsUpload { return { schema_version: 1, device, batch: batch(run) }; }
function adapter(db: DatabaseSync): MetricsConnection {
  return {
    execSync: sql => db.exec(sql),
    runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null,
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).all(...params) as T[],
  };
}
const accepted = (count = 1) => new Response(JSON.stringify({ status: 'accepted', experiment_id: 'ios-test', event_count: count }), { status: 202 });
function observation(batches: MetricBatch[]): Observation {
  return { snapshot: { schema_version: 1, ready: true, state: 'ready', capabilities: { whisper: true, whisper_metal: false, whisper_coreml: false, metrics_output: true } },
    observer: { capacity_events: 512, pending_events: 0, dropped_events: 0 }, batchesJSON: JSON.stringify(batches) };
}

test('outbox survives restart, preserves installation and rejects changed identities', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pheme-metric-test-'));
  let db = new DatabaseSync(join(directory, 'metrics.db'));
  try {
    let outbox = createMetricsOutbox(adapter(db));
    outbox.configure('http://192.168.1.40:8080/api/v1', true);
    const id = outbox.settings().installationID;
    outbox.enqueue([upload()]);
    const original = outbox.pending()[0];
    outbox.enqueue([upload()]);
    assert.equal(outbox.stats().pending, 1);
    const changed = upload(); changed.batch.events[0].value = 900;
    assert.throws(() => outbox.enqueue([changed]), /different content/);
    db.close();
    db = new DatabaseSync(join(directory, 'metrics.db'));
    outbox = createMetricsOutbox(adapter(db));
    assert.equal(outbox.settings().installationID, id);
    assert.equal(outbox.settings().url, 'http://192.168.1.40:8080');
    assert.deepEqual(outbox.pending()[0], original);
    outbox.acknowledge(original.id);
    outbox.acknowledge(original.id);
    assert.equal(outbox.stats().pending, 0);
    assert.equal(outbox.stats().acknowledged, 1);
  } finally { db.close(); rmSync(directory, { recursive: true, force: true }); }
});

test('network loss retries the exact saved payload and only acknowledges 202', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db), 'http://192.168.1.40:8080');
    outbox.enqueue([upload()]);
    const bodies: string[] = [];
    let offline = true;
    const sync = createMetricsSync(outbox, { device: async () => device, poll: async () => observation([]),
      fetch: async (_url, options) => {
        bodies.push(String(options?.body));
        if (offline) throw new Error('offline');
        return accepted();
      } });
    await sync.flush();
    assert.equal(outbox.stats().pending, 1);
    assert.match(sync.getSnapshot().error, /offline/);
    offline = false;
    await sync.flush();
    assert.equal(bodies[0], bodies[1]);
    assert.equal(outbox.stats().pending, 0);
    assert.equal(outbox.stats().acknowledged, 1);
  } finally { db.close(); }
});

test('paused uploads remain local and malformed acknowledgements keep the batch', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db));
    outbox.enqueue([upload()]);
    let calls = 0;
    const sync = createMetricsSync(outbox, { device: async () => device, poll: async () => observation([]),
      fetch: async () => { calls++; return new Response('{}', { status: 202 }); } });
    await sync.flush();
    assert.equal(calls, 0);
    sync.configure('http://192.168.1.40:8080', true);
    await sync.flush();
    assert.equal(outbox.stats().pending, 1);
    assert.match(sync.getSnapshot().error, /did not acknowledge/);
  } finally { db.close(); }
});

test('conflicting batches are retained as blocked without preventing later uploads', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db), 'http://192.168.1.40:8080');
    outbox.enqueue([upload('conflict'), upload('valid')]);
    const sync = createMetricsSync(outbox, { device: async () => device, poll: async () => observation([]),
      fetch: async (_url, options) => String(options?.body).includes('"run_id":"conflict"') ?
        new Response('{"error":"conflicting retry"}', { status: 409 }) : accepted() });
    await sync.flush();
    assert.equal(outbox.stats().blocked, 1);
    assert.equal(outbox.stats().pending, 0);
    assert.equal(outbox.stats().acknowledged, 1);
    assert.match(sync.getSnapshot().error, /conflicting retry/);
  } finally { db.close(); }
});

test('storage failure retains a drained observation and never drains it twice', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db));
    let failed = true, polls = 0;
    const sync = createMetricsSync({ ...outbox, enqueue: uploads => {
      if (failed) throw new Error('storage full');
      outbox.enqueue(uploads);
    } }, { device: async () => device, poll: async () => { polls++; return observation([batch()]); }, fetch: async () => accepted() });
    await sync.collect();
    assert.equal(polls, 1);
    assert.equal(outbox.stats().pending, 0);
    failed = false;
    await sync.collect();
    assert.equal(polls, 1);
    assert.equal(outbox.stats().pending, 1);
  } finally { db.close(); }
});

test('concurrent collectors/uploads have one drain and one in-flight sender', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db), 'http://192.168.1.40:8080');
    let polls = 0, uploads = 0;
    const sync = createMetricsSync(outbox, {
      device: async () => device, poll: async () => { polls++; return observation([batch()]); },
      fetch: async () => { uploads++; return accepted(); },
    });
    await Promise.all([sync.collect(), sync.collect()]);
    await Promise.all([sync.flush(), sync.flush()]);
    assert.equal(polls, 1); assert.equal(uploads, 1);
  } finally { db.close(); }
});

test('metadata failure retains host batches and does not drain native events', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db));
    let failed = true, polls = 0;
    const original = batch('host-retained');
    const sync = createMetricsSync(outbox, {
      device: async () => { if (failed) throw new Error('metadata unavailable'); return device; },
      poll: async () => { polls++; return observation([]); },
      fetch: async () => accepted(),
    });
    await sync.record(original);
    await sync.collect();
    assert.match(sync.getSnapshot().error, /metadata unavailable/);
    assert.equal(polls, 0);
    assert.equal(outbox.stats().pending, 0);
    failed = false;
    await sync.collect();
    assert.equal(polls, 1);
    assert.deepEqual(JSON.parse(outbox.pending()[0].payload).batch, original);
    assert.equal(sync.getSnapshot().droppedHostBatches, 0);
  } finally { db.close(); }
});

test('host retention is bounded and storage recovery persists the original batches', async () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db));
    let failed = true;
    const sync = createMetricsSync({ ...outbox, enqueue: uploads => {
      if (failed) throw new Error('storage unavailable');
      outbox.enqueue(uploads);
    } }, { device: async () => device, poll: async () => observation([]), fetch: async () => accepted() });
    for (let i = 0; i < 129; i++) await sync.record(batch(`host-${i}`));
    assert.equal(sync.getSnapshot().droppedHostBatches, 1);
    assert.equal(outbox.stats().pending, 0);
    failed = false;
    await sync.collect();
    assert.equal(outbox.stats().pending, 128);
    assert.deepEqual(JSON.parse(outbox.pending()[0].payload).batch, batch('host-0'));
  } finally { db.close(); }
});

test('bounded queue fails atomically without evicting unsent metrics', () => {
  const db = new DatabaseSync(':memory:');
  try {
    const outbox = createMetricsOutbox(adapter(db));
    outbox.enqueue([upload()]);
    assert.throws(() => outbox.enqueue(Array.from({ length: 2001 }, (_, i) => upload(`limit-${i}`))), /queue is full/);
    assert.equal(outbox.stats().pending, 1);
    assert.equal(outbox.pending()[0].payload, JSON.stringify(upload()));
  } finally { db.close(); }
});

test('server address validation allows local HTTP but rejects credentials and public HTTP', () => {
  assert.equal(normalizeMetricsURL('http://192.168.1.40:8080/api/v1/'), 'http://192.168.1.40:8080');
  assert.equal(normalizeMetricsURL('https://research.example.com'), 'https://research.example.com');
  for (const url of ['http://example.com', 'http://user:secret@192.168.1.40:8080', 'http://192.168.1.40:8080?token=x', 'file:///tmp', 'http://192.168.1.40:8080/other']) {
    assert.throws(() => normalizeMetricsURL(url));
  }
});

test('host metrics include timings/model identities without sending conversation content', () => {
  const model = { id: 'hash', sha256: 'a'.repeat(64), name: 'fixture-model', bytes: 1 };
  const asr = transcriptionMetadata({
    execution_location: 'device', model,
    settings: { modelID: model.id, language: 'en', dictionary: ['private vocabulary'], threads: 4, maxAudioSeconds: 120, metricsEnabled: true },
    transcription: { schema_version: 1, run_id: 'native-asr', result: { text: 'private transcript', status: 'accepted', model_id: model.id, processing_time_ms: 321 } },
  }, 'turn-1');
  const reply = replyMetrics({ execution_location: 'device', model,
    reply: { run_id: 'native-reply', text: 'private reply', processing_time_ms: 456 } }, 'turn-1');
  const json = JSON.stringify([asr, reply]);
  assert.doesNotMatch(json, /private transcript|private reply|private vocabulary/);
  assert.equal(reply.events.find(e => e.name === 'reasoning_duration_ms')?.value, 456);
  assert.equal(asr.events.find(e => e.name === 'asr_threads')?.unit, 'count');
  assert.equal(asr.events.find(e => e.name === 'native_run_id')?.value, 'native-asr');
  assert.equal(reply.events.find(e => e.name === 'turn_id')?.value, 'turn-1');
  assert.equal(reply.events.find(e => e.name === 'whole_device_power_watts')?.value, null);
  assert.ok(reply.events.find(e => e.name === 'whole_device_power_watts')?.unavailable_reason);
});

test('reply upload identities survive a native counter restarting at one', () => {
  const result = {
    execution_location: 'device' as const,
    model: { id: 'hash', sha256: 'a'.repeat(64), name: 'fixture-model', bytes: 1 },
    reply: { run_id: 'ios_reply_1', text: 'local reply', processing_time_ms: 123 },
  };
  const first = replyMetrics(result, 'local-1000-1');
  const restarted = replyMetrics(result, 'local-2000-1');
  assert.notEqual(first.run_id, restarted.run_id);
  assert.equal(first.events.find(event => event.name === 'native_run_id')?.value, 'ios_reply_1');
  for (const event of restarted.events) assert.equal(event.run_id, restarted.run_id);
});
