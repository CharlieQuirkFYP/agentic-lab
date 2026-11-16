import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { createBenchmarkStore } from '../src/benchmarkStore.ts';
import type { MetricsConnection } from '../src/metricsOutbox.ts';
import { createBenchmarkReport, projectObservations, reportToCSV, reportToJSON, reportToMarkdown } from '../../shared/phone-benchmarks/index.ts';

test('selected Results and all export formats retain one snapshot across arrivals and relaunch', () => {
  const db = new DatabaseSync(':memory:');
  const connection: MetricsConnection = {
    execSync: sql => db.exec(sql),
    runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).all(...params) as T[],
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null,
  };
  let serial = 0;
  const options = { installationID: 'results-fixture', uuid: () => `fixture-${++serial}`, now: () => 1000 + serial };
  const store = createBenchmarkStore(connection, options);
  try {
    store.setDevice({ installation_id: options.installationID, hardware: 'fixture-phone', os_version: 'fixture-os', app_version: 'fixture-app', build: 'fixture-build' });
    const configuration_id = store.registerConfiguration({ asr: null, reasoning: { id: 'fixture-model', name: 'Illustrative model', sha256: 'fixture-hash' } });
    const conversation_id = store.registerConversation({ configuration_id }, { source: 'manual' });
    const turn_id = store.registerTurn({ configuration_id, conversation_id }, { input_kind: 'text', turn_index: 0 });
    const context = { configuration_id, conversation_id, turn_id };
    const attempt_id = store.registerAttempt(context, { stage: 'reasoning', started_at_ms: 1000 });
    store.recordBatches([{ context: { ...context, attempt_id }, batch: {
      schema_version: 1, run_id: attempt_id, sequence_number: 0, timestamp_ms: 1000,
      events: [0, 12].map((value, sequence) => ({ schema_version: 1, run_id: attempt_id, sequence, timestamp_ms: 1000 + sequence * 500,
        name: 'resident_memory_bytes', value, unit: 'bytes', scope: 'process', source: 'fixture' })),
    } }]);
    store.completeAttempt(attempt_id, 'succeeded', { duration_ms: 500 });
    store.finalize({ ...context, attempt_id }, { status: 'complete', dropped_events: 0 });
    store.completeTurn(turn_id, 'succeeded');
    store.completeConversation(conversation_id, 'succeeded');
    store.finalize({ conversation_id }, { status: 'complete', dropped_events: 0 });

    const displayed = store.resultsSnapshot({ conversation_id });
    const visible = projectObservations(displayed.records, { filtered: true, selected_record_keys: displayed.selected_record_keys });
    const frozenIDs = displayed.records.map(item => item.record.record_id);
    const other = store.registerConversation({ configuration_id }, { source: 'manual' });
    store.registerTurn({ configuration_id, conversation_id: other }, { input_kind: 'text', turn_index: 0 });

    const report = createBenchmarkReport(displayed.records, { generated_at_ms: 2000, as_of: displayed.as_of,
      filters: { conversation_id }, input_already_filtered: true, selected_record_keys: displayed.selected_record_keys });
    assert.equal(report.projection.counts.conversations, visible.counts.conversations);
    assert.equal(report.projection.counts.turns, visible.counts.turns);
    assert.equal(report.projection.raw_metrics.length, 2);
    assert.equal(report.projection.attempts[0].duration_ms, 500);
    assert.equal(report.projection.attempts[0].completeness, 'complete');
    assert.deepEqual(displayed.records.map(item => item.record.record_id), frozenIDs);
    const json = JSON.parse(reportToJSON(report));
    assert.equal(json.input.as_of, displayed.as_of);
    assert.equal(json.input.filters.conversation_id, conversation_id);
    const rebuilt = projectObservations(json.projection.records, { filtered: true, selected_record_keys: json.input.selected_record_keys });
    assert.equal(rebuilt.raw_metrics[0].event.value, 0);
    const csv = reportToCSV(report);
    const markdown = reportToMarkdown(report).replace(/\\([\\`*_{}\[\]<>|])/g, '$1');
    assert.ok(csv.includes(displayed.as_of.replaceAll('"', '""')));
    assert.ok(markdown.includes(displayed.as_of));
    for (const output of [csv, markdown]) {
      assert.ok(output.includes(conversation_id));
      assert.ok(output.includes('resident_memory_bytes'));
    }
    const reopened = createBenchmarkStore(connection, options).resultsSnapshot({ conversation_id });
    assert.deepEqual(reopened.records.map(item => item.record.record_id), frozenIDs);
    assert.ok(reopened.sequence > displayed.sequence);
    const pending = store.resultsStatus();
    assert.equal(pending.pending, store.stats().pending);
    store.block([frozenIDs[0]], 'fixture');
    assert.equal(store.resultsStatus().blocked, 1);
    assert.equal(store.resultsStatus().pending, pending.pending - 1);
    store.acknowledge([frozenIDs[0]]);
    assert.equal(store.resultsStatus().blocked, 0);
    assert.ok(store.resultsStatus().lastUpload);
  } finally { db.close(); }
});
