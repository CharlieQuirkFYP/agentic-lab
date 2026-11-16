import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { createBenchmarkStore } from '../src/benchmarkStore.ts';
import { createBenchmarkCollector } from '../src/benchmarkCollector.ts';
import { projectObservations } from '../../shared/phone-benchmarks/index.ts';
import type { Inspection, LocalReply, NativeResourceSample, Observation } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { MetricBatch, MetricsConnection } from '../src/metricsOutbox.ts';

const device = { installation_id: 'fixture-install', hardware: 'fixture-phone', os_version: '1', app_version: '1', build: '1' };
const model = { id: 'model', name: 'Model', sha256: 'a'.repeat(64), bytes: 1 };
const snapshot = { schema_version: 1 as const, state: 'ready', ready: true, capabilities: { whisper: true, whisper_metal: false, whisper_coreml: false, metrics_output: true } };
const inspection: Inspection = { snapshot, busy: false, closed: false, model, reasoningModel: model,
  settings: { modelID: model.id, language: 'en', dictionary: ['private vocabulary'], threads: 2, maxAudioSeconds: 60, metricsEnabled: true }, reasoning: null, runtime_session_id: 'runtime-1' };
const observation = (samples: NativeResourceSample[] = []): Observation => ({ snapshot, batchesJSON: '[]', observer: { capacity_events: 512, pending_events: 0, dropped_events: 0 },
  runtime_session_id: 'runtime-1', resource_samples: samples, resource_observer: { capacity_samples: 512, pending_samples: 0, dropped_samples: 0, interval_ms: 500 } });
function adapter(db: DatabaseSync): MetricsConnection {
  return { execSync: sql => db.exec(sql), runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).all(...params) as T[],
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null };
}
function fixture(maxRecords = 10000) {
  const db = new DatabaseSync(':memory:');
  let serial = 0, time = 1000;
  const store = createBenchmarkStore(adapter(db), { installationID: device.installation_id, uuid: () => `id-${++serial}`, now: () => time++, maxRecords });
  store.setDevice(device);
  return { db, store };
}
function reply(id: string): LocalReply { return { execution_location: 'device', runtime_session_id: 'runtime-1', attempt_id: id, model, reply: { run_id: id, text: 'Private model response', processing_time_ms: 10 } }; }

test('live summary counts full history while returning only eight latest unique attempts',()=>{
  const {db,store}=fixture();
  const conversation=store.registerConversation({},{}),turn=store.registerTurn({conversation_id:conversation},{});
  for(let i=0;i<12;i++){
    const attempt=store.registerAttempt({conversation_id:conversation,turn_id:turn},{stage:'reasoning'});
    store.completeAttempt(attempt,'succeeded',{duration_ms:i});
  }
  store.recordBatches([{context:{conversation_id:conversation},batch:{schema_version:1,run_id:'live-fixture',sequence_number:0,timestamp_ms:1000,events:[{schema_version:1,run_id:'live-fixture',sequence:0,timestamp_ms:1000,name:'duration',value:0,unit:'ms',scope:'process',source:'fixture'}]}}]);
  const summary=store.liveSummary();
  assert.equal(summary.conversations,1);assert.equal(summary.turns,1);assert.equal(summary.attempts,12);assert.equal(summary.metric_events,1);
  assert.equal(summary.recent_attempts.length,8);assert.equal(new Set(summary.recent_attempts.map(a=>a.entity_id)).size,8);
  assert.equal(summary.recent_attempts[0].data.duration_ms,11);assert.ok(summary.recent_attempts.every(a=>a.data.status==='succeeded'));
  db.close();
});
function sample(id: string, time: number): NativeResourceSample {
  return { runtime_session_id: 'runtime-1', attempt_id: id, native_run_id: id, stage: 'reasoning', boundary: 'periodic', timestamp_ms: 1000.75 + time, monotonic_ms: time + 0.25, interval_ms: 500,
    values: [{ name: 'cpu_percent', value: time ? 0 : 20, unit: 'percent', scope: 'process', source: 'native.ios.resource' }] };
}

test('archive and pending marker commit atomically, acknowledgements preserve evidence', () => {
  const { db, store } = fixture(2);
  try {
    const conversation = store.registerConversation({}, { source: 'manual' });
    store.completeConversation(conversation, 'succeeded');
    assert.throws(() => store.registerTurn({ conversation_id: conversation }), (error: unknown) => (error as { code?: string }).code === 'archive_full');
    assert.equal(store.snapshot().length, 2); assert.equal(store.stats().pending, 2);
    const ids = store.pending().map(row => row.record_id);
    store.acknowledge(ids); store.acknowledge(ids);
    assert.equal(store.stats().pending, 0); assert.equal(store.snapshot().length, 2);
    assert.throws(() => store.completeConversation(conversation, 'running'), /cannot restart/);
  } finally { db.close(); }
});

test('configuration identities are stable and observation producer rejects private content', () => {
  const { db, store } = fixture();
  try {
    const config = { asr: model, reasoning: null, settings: { threads: 2 }, runtime: {} };
    const first = store.registerConfiguration(config);
    assert.equal(store.registerConfiguration({ runtime: {}, settings: { threads: 2 }, reasoning: null, asr: model }), first);
    assert.throws(() => store.append('turn', store.newID(), {}, { text: 'Private report' }), /Private content/);
    assert.throws(() => store.append('configuration', first, {}, { ...config, settings: { threads: 4 } }), /cannot change/);
  } finally { db.close(); }
});

test('unsynced, pinned and active history is protected, acknowledged closed history can be removed', () => {
  const { db, store } = fixture();
  try {
    const conversation = store.registerConversation({}, { source: 'manual' });
    assert.throws(() => store.removeAcknowledgedHistory(conversation), /Finish/);
    store.completeConversation(conversation, 'succeeded');
    assert.throws(() => store.removeAcknowledgedHistory(conversation), /Pending/);
    store.acknowledge(store.pending().map(row => row.record_id)); store.pin(conversation, true);
    assert.throws(() => store.removeAcknowledgedHistory(conversation), /Unpin/);
    store.pin(conversation, false); store.removeAcknowledgedHistory(conversation);
    assert.equal(store.snapshot().length, 0);
  } finally { db.close(); }
});

test('runtime recovery explicitly interrupts attempts and conversations while preserving resumable jobs', () => {
  const { db, store } = fixture();
  try {
    const sweep = store.registerSweep({ status: 'running', source: 'sweep' });
    const job = store.registerJob({ sweep_id: sweep }, { status: 'running', source: 'sweep' });
    const conversation = store.registerConversation({ job_id: job, sweep_id: sweep, runtime_session_id: 'old' }, { source: 'sweep' });
    const attempt = store.registerAttempt({ conversation_id: conversation, job_id: job, runtime_session_id: 'old' }, { status: 'running', stage: 'reasoning', source: 'sweep' });
    store.recoverInterrupted('runtime_replaced', 'new');
    assert.equal(store.latest('attempt', attempt)?.data.status, 'interrupted');
    assert.equal(store.latest('conversation', conversation)?.data.status, 'interrupted');
    assert.equal(store.latest('job', job)?.data.status, 'running');
    assert.equal(store.latest('job', job)?.data.next_execution_required, true);
    assert.equal(store.latest('finalization', attempt)?.data.status, 'partial');
  } finally { db.close(); }
});

test('multiple resource samples have unique stable events and integer wire timestamps', async () => {
  const { db, store } = fixture();
  try {
    let samples: NativeResourceSample[] = [];
    let mono = 1;
    const collector = createBenchmarkCollector(store, { device: async () => device, inspect: async () => inspection,
      poll: async () => { mono += 10000; const batch = samples; samples = []; return observation(batch); }, cancel: async () => {},
      settings: () => ({ url: '', enabled: false, installationID: device.installation_id }), fetch, monotonicNow: () => mono });
    const turn = await collector.beginManualTurn('text');
    await collector.manualAttempt(turn, 'reasoning', async id => { samples = [sample(id, 0), sample(id, 500)]; mono += 20; return reply(id); });
    collector.finishManualTurn(turn, 'succeeded'); collector.closeManualConversation();
    const projection = projectObservations(store.snapshot());
    assert.equal(projection.attempts[0].duration_ms, 20);
    const cpu = projection.metric_groups.find(group => group.name === 'cpu_percent')!;
    assert.equal(cpu.numeric.count, 2); assert.equal(cpu.numeric.min, 0);
    assert.equal(projection.conversations[0].completeness, 'complete');
    for (const { record } of store.snapshot().filter(item => item.record.kind === 'metric_batch')) {
      const batch = record.data.batch as MetricBatch;
      assert.ok(Number.isInteger(batch.timestamp_ms));
      for (const event of batch.events) assert.ok(Number.isInteger(event.timestamp_ms));
    }
    assert.doesNotMatch(JSON.stringify(store.snapshot()), /Private model response|private vocabulary/);
  } finally { db.close(); }
});

test('cancellation holds ownership and persists no terminal inference result until native settles', async () => {
  const { db, store } = fixture();
  try {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const collector = createBenchmarkCollector(store, { device: async () => device, inspect: async () => inspection, poll: async () => observation(), cancel: async () => {},
      settings: () => ({ url: '', enabled: false, installationID: device.installation_id }), fetch });
    const turn = await collector.beginManualTurn('text');
    const attempt = store.registerAttempt({ ...store.latest('turn', turn)!.context, turn_id: turn }, { source: 'manual', stage: 'reasoning' });
    const operation = collector.runOperation({ owner: 'manual', attemptID: attempt, stage: 'reasoning' }, async id => { await gate; return reply(id); });
    await collector.interrupt('user_cancelled');
    assert.equal(store.latest('attempt', attempt)?.data.status, 'running');
    await assert.rejects(collector.withOwnership('sweep', async () => {}), /owns/);
    release(); await assert.rejects(operation, /cancelled/);
    assert.equal(store.latest('attempt', attempt)?.data.status, 'cancelled');
    assert.equal(collector.getOwner(), null);
  } finally { db.close(); }
});

test('collector retains exact drained batches across SQLite failure without draining twice', async () => {
  const { db, store } = fixture();
  try {
    let reject = true, polls = 0;
    const collector = createBenchmarkCollector({ ...store, recordBatches: batches => { if (reject) throw new Error('disk full'); return store.recordBatches(batches); } }, {
      device: async () => device, inspect: async () => inspection, poll: async () => { polls++; return observation([sample('unresolved-attempt', 500)]); }, cancel: async () => {},
      settings: () => ({ url: '', enabled: false, installationID: device.installation_id }), fetch });
    await assert.rejects(collector.collect(), /disk full/); assert.equal(polls, 1);
    reject = false; await collector.collect(); assert.equal(polls, 1);
    assert.equal(store.snapshot().filter(item => item.record.kind === 'metric_batch').length, 1);
  } finally { db.close(); }
});

test('offline delivery retries byte-identical chunks and validates every acknowledgement ID', async () => {
  const { db, store } = fixture();
  try {
    const conversation = store.registerConversation({}, { source: 'manual' }); store.completeConversation(conversation, 'succeeded');
    const bodies: string[] = []; let mode = 'offline';
    const collector = createBenchmarkCollector(store, { device: async () => device, inspect: async () => inspection, poll: async () => observation(), cancel: async () => {},
      settings: () => ({ url: 'http://localhost:8080', enabled: true, installationID: device.installation_id }),
      fetch: async (_url, options) => {
        const body = String(options?.body); bodies.push(body);
        if (mode === 'offline') throw new Error('offline');
        const records = JSON.parse(body).records as { record_id: string }[];
        return new Response(JSON.stringify({ status: 'accepted', record_count: records.length, accepted_record_ids: mode === 'bad' ? ['wrong'] : records.map(record => record.record_id) }), { status: 202 });
      } });
    await collector.flush(); mode = 'bad'; await collector.flush(); assert.equal(store.stats().pending, 2);
    mode = 'ok'; await collector.flush(); assert.equal(store.stats().pending, 0); assert.equal(store.snapshot().length, 2);
    assert.equal(bodies[0], bodies[1]); assert.equal(bodies[1], bodies[2]);
  } finally { db.close(); }
});

test('capacity headroom refuses admission before native callback executes', async () => {
  const { db, store } = fixture(20);
  try {
    const collector = createBenchmarkCollector(store, { device: async () => device, inspect: async () => inspection, poll: async () => observation(), cancel: async () => {},
      settings: () => ({ url: '', enabled: false, installationID: device.installation_id }), fetch });
    const turn = await collector.beginManualTurn('text'); let nativeCalls = 0;
    await assert.rejects(collector.manualAttempt(turn, 'reasoning', async id => { nativeCalls++; return reply(id); }), (cause: unknown) => (cause as { code?: string }).code === 'archive_full');
    assert.equal(nativeCalls, 0); assert.equal(store.snapshot().find(item => item.record.kind === 'attempt')?.record.data.status, 'planned');
  } finally { db.close(); }
});

test('manual chat keeps one conversation through reviewed and typed turns, reset starts a new one', async () => {
  const { createLocalChat } = await import('../src/localChat.ts');
  const { db, store } = fixture();
  try {
    let calls = 0, persisted = 0;
    const collector = createBenchmarkCollector(store, { device: async () => device, inspect: async () => inspection, poll: async () => observation(), cancel: async () => {},
      settings: () => ({ url: '', enabled: false, installationID: device.installation_id }), fetch });
    const client = {
      respond: async (_text: string, _request: string, id: string) => { assert.equal(store.latest('attempt', id)?.data.status, 'running'); calls++; return reply(id); },
      transcribeFile: async (_uri: string, id: string) => {
        assert.equal(store.latest('attempt', id)?.data.status, 'running'); calls++;
        return { execution_location: 'device' as const, model, settings: inspection.settings!, transcription: { schema_version: 1 as const, run_id: id,
          result: { text: 'Private transcript', status: 'accepted', model_id: model.id, processing_time_ms: 12 } } };
      }, resetConversation: async () => {},
    } as unknown as Parameters<typeof createLocalChat>[0];
    const chat = createLocalChat(client, () => { persisted++; }, undefined, collector);
    let turnID = '';
    await chat.startTurn({ uri: 'file:///private.wav' }, event => { turnID = String(event.data.turn_id); });
    assert.equal((await chat.getTurn(turnID)).status, 'awaiting_review');
    await assert.rejects(collector.withOwnership('sweep', async () => {}), /reviewed manual turn/);
    await chat.submitTurn(turnID, 'Private correction');
    await chat.startTurn('Private followup', () => {});
    let projection = projectObservations(store.snapshot());
    assert.equal(calls, 3); assert.equal(persisted, 1);
    assert.equal(projection.conversations.length, 1); assert.equal(projection.conversations[0].status, 'active');
    assert.equal(projection.conversations[0].turns.length, 2); assert.equal(projection.attempts.length, 3);
    await chat.resetChat();
    projection = projectObservations(store.snapshot());
    assert.equal(projection.conversations[0].status, 'succeeded'); assert.equal(projection.conversations[0].completeness, 'complete');
    await chat.startTurn('Private new conversation', () => {});
    assert.equal(projectObservations(store.snapshot()).conversations.length, 2);
    assert.doesNotMatch(JSON.stringify(store.snapshot()), /Private transcript|Private correction|Private followup|private.wav/);
  } finally { db.close(); }
});
