import assert from 'node:assert/strict';
import test from 'node:test';
import { performance } from 'node:perf_hooks';
import { DatabaseSync } from 'node:sqlite';
import { createBenchmarkStore } from '../src/benchmarkStore.ts';
import type { ResultsHistoryOptions } from '../src/benchmarkStore.ts';
import type { MetricsConnection } from '../src/metricsOutbox.ts';
import { projectObservations } from '../../shared/phone-benchmarks/index.ts';
import type { PhoneObservation, PhoneObservationContext } from '../../shared/phone-benchmarks/types.ts';

const device = { installation_id: 'results-install', hardware: 'fixture-phone', os_version: '1', app_version: '1', build: '1' };
function fixture() {
  const db = new DatabaseSync(':memory:');
  const reads: { sql: string; rows: number }[] = [];
  const connection: MetricsConnection = {
    execSync: sql => db.exec(sql), runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => {
      const rows = db.prepare(sql).all(...params) as T[]; reads.push({ sql, rows: rows.length }); return rows;
    },
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null,
  };
  let serial = 0, time = 1000;
  const options = { installationID: device.installation_id, uuid: () => `fixture-${++serial}`, now: () => time++ };
  const store = createBenchmarkStore(connection, options);
  store.setDevice(device);
  return { db, store, connection, options, reads };
}

test('Results history pages latest conversations with full filtered totals and active-first ordering', () => {
  const { db, store } = fixture();
  try {
    const small = store.registerConfiguration({ asr: { name: 'Whisper tiny' }, reasoning: { name: 'Qwen small' } });
    const large = store.registerConfiguration({ asr: { name: 'Whisper large' }, reasoning: { name: 'Qwen large' } });
    const sweep_id = store.registerSweep({ source: 'sweep', status: 'failed' });
    const expected: string[] = [];
    for (let index = 0; index < 67; index++) {
      const source = index < 55 ? 'manual' : index < 61 ? 'sweep' : 'acceptance';
      const id = store.registerConversation({ configuration_id: index % 2 ? small : large, ...(source === 'manual' ? {} : { sweep_id }) },
        { source, started_at_ms: index * 100 });
      store.completeConversation(id, 'succeeded');
      if (source === 'manual') expected.unshift(id);
    }
    const active = store.registerConversation({ configuration_id: small }, { source: 'manual', started_at_ms: 1 });
    expected.unshift(active);
    const first = store.resultsHistory({ source: 'manual' });
    assert.equal(first.total, 56); assert.equal(first.items.length, 20);
    assert.equal(first.items[0].entity_id, active); assert.ok(first.next_cursor);
    const ids = first.items.map(item => item.entity_id);
    let cursor: string | null = first.next_cursor;
    while (cursor) {
      const page = store.resultsHistory({ source: 'manual', cursor });
      assert.equal(page.total, 56); ids.push(...page.items.map(item => item.entity_id)); cursor = page.next_cursor;
    }
    assert.deepEqual(ids, expected); assert.equal(new Set(ids).size, 56);
    assert.equal(store.resultsHistory({ limit: 999 }).items.length, 50);
    assert.equal(store.resultsHistory({ limit: 0 }).items.length, 1);
    const filtered = store.resultsHistory({ source: 'manual', query: 'TINY', since_ms: 1000, until_ms: 3000, limit: 3 });
    assert.equal(filtered.total, 10); assert.equal(filtered.items.length, 3);
    assert.ok(filtered.items.every(item => item.asr_name === 'Whisper tiny' && item.reasoning_name === 'Qwen small' && item.configuration_id === small));
    assert.equal(store.resultsHistory({ sweep_id }).total, 12);
    assert.equal(store.resultsHistory({ source: 'sweep', sweep_id }).total, 6);
    assert.equal(store.resultsHistory({ source: 'acceptance', sweep_id }).total, 6);
    assert.ok(store.resultsHistory({ source: 'acceptance' }).items.every(item => item.source === 'acceptance'));
    assert.equal(store.resultsHistory({ query: '%' }).total, 0, 'search treats SQL wildcard characters literally');
    assert.throws(() => store.resultsHistory({ source: 'acceptance', cursor: first.next_cursor! }), /Invalid Results/);
    assert.throws(() => store.resultsHistory({ cursor: 'not-json' }), /Invalid Results/);
    assert.ok(Object.isFrozen(first)); assert.ok(Object.isFrozen(first.items[0]));
  } finally { db.close(); }
});

test('history cursor holds its sequence watermark through late writes and outcome changes', () => {
  const { db, store } = fixture();
  try {
    const ids = Array.from({ length: 5 }, (_, index) => store.registerConversation({}, { source: 'manual', started_at_ms: index * 100 }));
    const options: ResultsHistoryOptions = { source: 'manual', limit: 2 };
    const first = store.resultsHistory(options);
    store.completeConversation(ids[3], 'succeeded');
    store.registerConversation({}, { source: 'manual', started_at_ms: 9999 });
    const second = store.resultsHistory({ ...options, cursor: first.next_cursor! });
    const third = store.resultsHistory({ ...options, cursor: second.next_cursor! });
    assert.equal(second.total, 5); assert.equal(third.total, 5);
    assert.deepEqual([...first.items, ...second.items, ...third.items].map(item => item.entity_id), ids.reverse());
    assert.equal(store.resultsHistory(options).total, 6);
  } finally { db.close(); }
});

test('scoped snapshots resolve indirect children, include ancestors and finalizations, and exclude sibling evidence', () => {
  const { db, store, reads } = fixture();
  try {
    const configuration_id = store.registerConfiguration({ reasoning: { name: 'Target model' } });
    const otherConfiguration = store.registerConfiguration({ reasoning: { name: 'Other model' } });
    const sweep_id = store.registerSweep({ source: 'sweep', status: 'running' });
    function conversation(configuration: string, complete: boolean) {
      const job_id = store.registerJob({ sweep_id, configuration_id: configuration }, { source: 'sweep', status: 'running' });
      const conversation_id = store.registerConversation({ job_id }, { source: 'sweep' });
      const turn_id = store.registerTurn({ conversation_id }, { status: 'running' });
      const attempt_id = store.registerAttempt({ turn_id }, { status: 'running', stage: 'reasoning' });
      const batch = store.recordBatches([{ context: { attempt_id }, batch: {
        schema_version: 1, run_id: attempt_id, sequence_number: 0, timestamp_ms: 1000,
        events: [{ schema_version: 1, run_id: attempt_id, sequence: 0, timestamp_ms: 1000, name: 'cpu_percent', value: 12, unit: 'percent_one_core', scope: 'process', source: 'fixture' }],
      } }])[0];
      store.completeAttempt(attempt_id, 'succeeded', { duration_ms: 10 });
      const final = store.finalize({ attempt_id }, { status: 'complete', dropped_events: 0 });
      if (complete) {
        store.completeTurn(turn_id, 'succeeded'); store.completeConversation(conversation_id, 'succeeded'); store.completeJob(job_id, 'succeeded');
      }
      return { conversation_id, job_id, turn_id, attempt_id, batch, final };
    }
    const target = conversation(configuration_id, true), sibling = conversation(otherConfiguration, true);
    const direct = store.registerConversation({ sweep_id, configuration_id: otherConfiguration }, { source: 'sweep', status: 'succeeded' });
    store.completeSweep(sweep_id, 'succeeded');
    const sweepFinal = store.finalize({ sweep_id }, { status: 'complete', dropped_events: 0 });
    reads.length = 0;
    const snapshot = store.resultsSnapshot({ conversation_id: target.conversation_id });
    const selectedIDs = new Set(snapshot.selected_record_keys.map(ref => ref.record_id));
    const records = snapshot.records.map(item => item.record);
    const recordIDs = new Set(records.map(record => record.record_id));
    for (const id of [target.conversation_id, target.turn_id, target.attempt_id, target.job_id, configuration_id, sweep_id]) assert.ok(records.some(record => record.entity_id === id));
    assert.ok(recordIDs.has(target.batch.record_id)); assert.ok(recordIDs.has(target.final.record_id));
    assert.ok(!recordIDs.has(sibling.batch.record_id)); assert.ok(!recordIDs.has(sweepFinal.record_id));
    assert.ok(!records.some(record => record.entity_id === direct));
    assert.ok(selectedIDs.has(target.final.record_id));
    assert.ok(records.filter(record => ['configuration', 'job', 'sweep'].includes(record.kind)).every(record => !selectedIDs.has(record.record_id)));
    const projection = projectObservations(snapshot.records, { filtered: true });
    assert.equal(projection.conversations.length, 1); assert.equal(projection.raw_metrics.length, 1);
    assert.equal(projection.attempts[0].completeness, 'complete');
    assert.equal(projection.unresolved_references.length, 0);
    assert.equal(projection.attempts[0].context.configuration_id, configuration_id);
    assert.equal(reads.filter(read => /a\.payload/.test(read.sql)).length, 1);
    assert.equal(reads.find(read => /a\.payload/.test(read.sql))!.rows, snapshot.records.length);
    const configuration = store.resultsSnapshot({ sweep_id, configuration_id });
    assert.deepEqual(configuration.records.map(item => item.record.record_id), snapshot.records.map(item => item.record.record_id));
    const completeSweep = store.resultsSnapshot({ sweep_id });
    assert.ok(completeSweep.records.some(item => item.record.record_id === sweepFinal.record_id));
    assert.ok(completeSweep.records.some(item => item.record.record_id === sibling.batch.record_id));
    assert.equal(store.resultsSnapshot({ conversation_id: target.conversation_id, configuration_id: otherConfiguration }).records.length, 0);
    assert.equal(store.resultsSnapshot({}).records.length, 0);
    assert.equal(store.resultsSnapshot({ conversation_id: 'missing' }).records.length, 0);
  } finally { db.close(); }
});

test('immutable read snapshots and durable generations survive writes, protected removal and store reopening', () => {
  const { db, store, connection, options } = fixture();
  try {
    const conversation_id = store.registerConversation({}, { source: 'manual' });
    store.completeConversation(conversation_id, 'succeeded');
    let publications = 0;
    const unsubscribe = store.subscribe(() => { publications++; });
    const snapshot = store.resultsSnapshot({ conversation_id });
    const retained = JSON.stringify(snapshot);
    store.resultsHistory(); store.resultsSnapshot({ conversation_id });
    assert.equal(publications, 0, 'Results reads must not publish mutations');
    assert.ok(Object.isFrozen(snapshot)); assert.ok(Object.isFrozen(snapshot.records));
    assert.ok(Object.isFrozen(snapshot.records[0].record.data));
    assert.throws(() => { snapshot.records[0].record.data.status = 'changed'; }, TypeError);
    assert.throws(() => store.removeAcknowledgedHistory(conversation_id), /Pending/);
    assert.equal(store.resultsSnapshot({ conversation_id }).as_of, snapshot.as_of, 'rolled-back removal does not change its token');
    store.block(store.pending().map(row => row.record_id), 'fixture');
    assert.throws(() => store.removeAcknowledgedHistory(conversation_id), /Pending/);
    assert.equal(store.stats().blocked, 2);
    store.retryBlocked(); assert.equal(store.stats().pending, 2);
    store.acknowledge(store.pending().map(row => row.record_id)); store.pin(conversation_id, true);
    assert.throws(() => store.removeAcknowledgedHistory(conversation_id), /Unpin/);
    assert.equal(store.resultsHistory().total, 1); assert.ok(store.isPinned(conversation_id));
    store.pin(conversation_id, false);
    const beforeRemoval = store.resultsSnapshot({ conversation_id });
    const unchanged = createBenchmarkStore(connection, options).resultsSnapshot({ conversation_id });
    assert.equal(unchanged.as_of, beforeRemoval.as_of, 'reopening alone keeps the durable token');
    store.removeAcknowledgedHistory(conversation_id);
    const reopened = createBenchmarkStore(connection, options).resultsSnapshot({ conversation_id });
    assert.equal(reopened.sequence, beforeRemoval.sequence);
    assert.ok(reopened.generation > beforeRemoval.generation); assert.notEqual(reopened.as_of, beforeRemoval.as_of);
    assert.equal(reopened.records.length, 0); assert.equal(store.resultsHistory().total, 0);
    assert.equal(JSON.stringify(snapshot), retained, 'a retained input is independent of removed rows');
    const next = store.registerConversation({}, { source: 'manual' });
    const later = store.resultsSnapshot({ conversation_id: next });
    assert.ok(later.sequence > reopened.sequence); assert.notEqual(later.as_of, reopened.as_of);
    unsubscribe();
  } finally { db.close(); }
});

test('late metrics and newer finalizations cannot rewrite a displayed complete result', () => {
  const { db, store } = fixture();
  try {
    const conversation_id = store.registerConversation({}, { source: 'manual' });
    const attempt_id = store.registerAttempt({ conversation_id }, { stage: 'reasoning', status: 'running' });
    const recordContext = { conversation_id, attempt_id };
    const batch = (sequence_number: number) => ({ context: recordContext, batch: {
      schema_version: 1 as const, run_id: attempt_id, sequence_number, timestamp_ms: 1000,
      events: [{ schema_version: 1 as const, run_id: attempt_id, sequence: sequence_number, timestamp_ms: 1000,
        name: 'cpu_percent', value: sequence_number, unit: 'percent_one_core', scope: 'process', source: 'fixture' }],
    } });
    store.recordBatches([batch(0)]); store.completeAttempt(attempt_id, 'succeeded', { duration_ms: 15 });
    store.finalize(recordContext, { status: 'complete', dropped_events: 0 });
    const displayed = store.resultsSnapshot({ conversation_id });
    const oldProjection = projectObservations(displayed.records, { filtered: true });
    assert.equal(oldProjection.attempts[0].completeness, 'complete');
    assert.equal(oldProjection.raw_metrics.length, 1);
    store.recordBatches([batch(1)]);
    store.finalize(recordContext, { status: 'complete', dropped_events: 0 });
    const refreshed = store.resultsSnapshot({ conversation_id });
    assert.ok(refreshed.sequence > displayed.sequence); assert.notEqual(refreshed.as_of, displayed.as_of);
    assert.equal(projectObservations(refreshed.records, { filtered: true }).raw_metrics.length, 2);
    const stable = projectObservations(displayed.records, { filtered: true });
    assert.equal(stable.raw_metrics.length, 1); assert.equal(stable.attempts[0].completeness, 'complete');
    assert.equal(stable.records.filter(item => item.record.kind === 'finalization').length, 1);
  } finally { db.close(); }
});

test('near-capacity archive reads cross the bridge only for a history page and selected conversation', context => {
  const { db, store, reads } = fixture();
  try {
    const configuration_id = store.registerConfiguration({ asr: { name: 'Fixture speech' }, reasoning: { name: 'Fixture reply' } });
    const stream = db.prepare("SELECT value FROM phone_observation_state WHERE key='stream'").get()!.value as string;
    const insert = db.prepare('INSERT INTO phone_observation_archive VALUES(?,?,?,?,?,?,?,?,?)');
    const serializedDevice = JSON.stringify(device);
    let sequence = 1;
    const padding = 'x'.repeat(410);
    const add = (kind: PhoneObservation['kind'], entity_id: string, revision: number, recordContext: PhoneObservationContext, data: Record<string, unknown>) => {
      sequence++;
      const record: PhoneObservation = { schema_version: 1, record_id: `capacity-record-${sequence}`, producer_stream_id: stream,
        record_sequence: sequence, timestamp_ms: sequence, kind, entity_id, revision, context: recordContext, data: { ...data, fixture_padding: padding } };
      insert.run(record.record_id, stream, sequence, kind, entity_id, revision, JSON.stringify(record), serializedDevice, sequence);
      return record.record_id;
    };
    db.exec('BEGIN');
    for (let index = 0; index < 4800; index++) {
      const conversation_id = `capacity-conversation-${index}`, turn_id = `capacity-turn-${index}`, attempt_id = `capacity-attempt-${index}`;
      const parent = { configuration_id, conversation_id }, child = { ...parent, turn_id }, attempt = { ...child, attempt_id };
      add('conversation', conversation_id, 1, { configuration_id }, { source: 'manual', status: 'active', started_at_ms: index });
      add('turn', turn_id, 1, parent, { status: 'running', turn_index: 0 });
      add('attempt', attempt_id, 1, child, { status: 'running', stage: 'reasoning' });
      for (let batch = 0; batch < 2; batch++) add('metric_batch', `capacity-batch-${index}-${batch}`, 1, attempt, { batch: {
        schema_version: 1, run_id: attempt_id, sequence_number: batch, timestamp_ms: sequence,
        events: [{ schema_version: 1, run_id: attempt_id, sequence: batch, timestamp_ms: sequence, name: 'resident_memory_bytes', value: index, unit: 'bytes', scope: 'process', source: 'fixture' }],
      } });
      add('attempt', attempt_id, 2, child, { status: 'succeeded', stage: 'reasoning', duration_ms: 12 });
      add('finalization', attempt_id, 1, attempt, { status: 'partial', event_count: 2 });
      add('turn', turn_id, 2, parent, { status: 'succeeded', turn_index: 0 });
      add('conversation', conversation_id, 2, { configuration_id }, { source: 'manual', status: 'succeeded', started_at_ms: index });
      add('finalization', conversation_id, 1, parent, { status: 'partial', event_count: 2 });
    }
    db.prepare("UPDATE phone_observation_state SET value=? WHERE key='sequence'").run(String(sequence));
    db.exec('COMMIT');
    const stats = store.stats();
    assert.ok(stats.records >= 50000 * 0.95 && stats.records <= 50000);
    assert.ok(stats.bytes >= 48 * 1024 * 1024 * 0.9 && stats.bytes <= 48 * 1024 * 1024, `${stats.bytes} archive bytes`);
    reads.length = 0;
    const started = performance.now();
    const page = store.resultsHistory({ source: 'manual' });
    const historyMs = performance.now() - started;
    const detailStarted = performance.now();
    const snapshot = store.resultsSnapshot({ conversation_id: 'capacity-conversation-4799' });
    const detailMs = performance.now() - detailStarted;
    assert.equal(page.items.length, 20); assert.equal(page.total, 4800);
    assert.equal(snapshot.records.length, 11); assert.equal(snapshot.selected_record_keys.length, 10);
    assert.ok(reads.every(read => read.rows <= 21), 'no full-archive payload crosses the SQLite bridge');
    context.diagnostic(`Node SQLite fixture: ${stats.records} records, ${(stats.bytes / 1024 / 1024).toFixed(2)} MiB; history ${historyMs.toFixed(1)} ms; scoped read ${detailMs.toFixed(1)} ms; 21 history rows / 11 scoped rows returned. Device responsiveness remains a physical acceptance gate.`);
  } finally { db.close(); }
});
