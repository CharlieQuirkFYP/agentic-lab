import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import test from 'node:test';
import {
  createBenchmarkReport, projectObservations, reportToCSV, reportToJSON, reportToMarkdown,
  type BenchmarkReport, type ExecutionStatus, type ObservationFilters, type PhoneObservation,
  type PhoneObservationPage, type RecordReference, type StoredPhoneObservation,
} from '../../shared/phone-benchmarks/index.ts';
import { createBenchmarkStore, type BenchmarkStore } from '../src/benchmarkStore.ts';
import type { MetricBatch, MetricsConnection } from '../src/metricsOutbox.ts';

const target = process.env.PHEME_BENCHMARK_TEST_URL;
const fixtureSource = 'http_integration_fixture';
const fixtureModels = ['Fixture Whisper A', 'Fixture Qwen A', 'Fixture Whisper B', 'Fixture Qwen B'];

interface Overview {
  as_of: string;
  source: string;
  coverage: {
    record_count: number; metric_batch_count: number; event_count: number;
    legacy_batch_count: number; legacy_conversation_count: null; unresolved_record_count: number;
    finalization: { complete: number; partial: number; unknown: number; dropped_events: number };
  };
  counts: {
    configurations: number;
    conversations: { total: number; outcomes: Record<string, number> };
    turns: { total: number; outcomes: Record<string, number> };
    attempts: { total: number; outcomes: Record<string, number> };
  };
  series: {
    configuration_id: string; stage: string; outcome: string; name: string;
    source: string; numeric_count: number; mean: number | null; min: number | null;
    unavailable_count: number; status_counts: Record<string, number>;
    configuration: { asr: { name: string }; reasoning: { name: string } };
  }[];
  current_conversation: { entity_id: string } | null;
  recent_conversations: { entity_id: string; completeness: string }[];
  recent_runs: unknown[];
}

function connection(db: DatabaseSync): MetricsConnection {
  return {
    execSync: sql => db.exec(sql),
    runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null,
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).all(...params) as T[],
  };
}

function populate(store: BenchmarkStore) {
  const configurations = [0, 1].map(index => store.registerConfiguration({
    asr: { id: `fixture-asr-${index}`, name: fixtureModels[index * 2], sha256: String(index + 1).repeat(64) },
    reasoning: { id: `fixture-reasoning-${index}`, name: fixtureModels[index * 2 + 1], sha256: String(index + 3).repeat(64) },
    settings: { threads: 2, temperature: 0, fixture_only: true },
    runtime: { implementation: 'http-fixture-no-inference', revision: 'fixture-v1' },
  }));
  const conversations: { id: string; status: ExecutionStatus; configuration: string; source: string }[] = [];
  const outcomes: ExecutionStatus[] = [...Array<ExecutionStatus>(20).fill('succeeded'), 'failed', 'cancelled', 'interrupted', 'skipped'];
  for (const [index, status] of outcomes.entries()) {
    const configuration = configurations[index < 21 ? 0 : 1];
    const source = index < 21 ? 'manual' : 'acceptance';
    const context = { configuration_id: configuration, runtime_session_id: store.newID() };
    const conversationID = store.registerConversation(context, { source, case_id: `fixture-case-${index}`, dataset_revision: 'fixture-v1', protocol_revision: 'fixture-v1' });
    conversations.push({ id: conversationID, status, configuration, source });
    for (let turnIndex = 0; turnIndex < 2; turnIndex++) {
      const turnID = store.registerTurn({ ...context, conversation_id: conversationID }, { status: 'running', turn_index: turnIndex });
      for (const stage of ['transcription', 'reasoning']) {
        const attemptContext = { ...context, conversation_id: conversationID, turn_id: turnID };
        const runID = `fixture-run-${index}-${turnIndex}-${stage}`;
        const attemptID = store.registerAttempt(attemptContext, { status: status === 'skipped' ? 'planned' : 'running', stage, native_run_id: runID });
        let eventCount = 0;
        if (status !== 'skipped') {
          const timestamp = 10_000 + index * 100 + turnIndex;
          const event = { schema_version: 1 as const, run_id: runID, timestamp_ms: timestamp, source: fixtureSource };
          const batch: MetricBatch = {
            schema_version: 1, run_id: runID, sequence_number: 0, timestamp_ms: timestamp,
            events: [
              { ...event, sequence: 0, name: 'duration_ms', value: index * 10 + turnIndex, unit: 'milliseconds', scope: 'run' },
              { ...event, sequence: 1, name: 'cpu_percent', value: 0, unit: 'percent', scope: 'process' },
              { ...event, sequence: 2, name: 'whole_device_power_watts', value: null, unit: 'watts', scope: 'device', unavailable_reason: 'fixture_has_no_power_meter' },
              { ...event, sequence: 3, name: 'thermal_state', value: 'fixture_unknown', unit: 'state', scope: 'device' },
            ],
          };
          store.recordBatches([{ batch, context: { ...attemptContext, attempt_id: attemptID } }]);
          eventCount = batch.events.length;
        }
        store.completeAttempt(attemptID, status, { duration_ms: index * 10 + turnIndex });
        store.finalize({ ...attemptContext, attempt_id: attemptID }, {
          status: status === 'interrupted' ? 'partial' : 'complete', event_count: eventCount,
          dropped_events: status === 'interrupted' ? 1 : 0,
        });
      }
      store.completeTurn(turnID, status, { duration_ms: index * 20 + turnIndex });
    }
    store.completeConversation(conversationID, status, { duration_ms: index * 40 });
    store.finalize({ ...context, conversation_id: conversationID }, {
      status: status === 'interrupted' ? 'partial' : 'complete',
      event_count: status === 'skipped' ? 0 : 16, dropped_events: status === 'interrupted' ? 4 : 0,
    });
  }
  return { configurations, conversations };
}

function compareReport(overview: Overview, records: StoredPhoneObservation[], asOf: string, filters: ObservationFilters = {}, selectedKeys?: RecordReference[]): BenchmarkReport {
  const report = createBenchmarkReport(records, { generated_at_ms: 100_000, as_of: asOf, filters, input_already_filtered: true, selected_record_keys: selectedKeys });
  assert.equal(overview.as_of, asOf);
  assert.equal(overview.source, 'device_reported');
  assert.equal(overview.coverage.record_count, report.totals.canonical_records);
  assert.equal(overview.coverage.metric_batch_count, report.totals.metric_batches);
  assert.equal(overview.coverage.event_count, report.totals.metric_events);
  assert.equal(overview.coverage.legacy_batch_count, 0);
  assert.equal(overview.coverage.legacy_conversation_count, null);
  assert.equal(overview.counts.configurations, report.projection.counts.configurations);
  for (const [plural, singular] of [['conversations', 'conversation'], ['turns', 'turn'], ['attempts', 'attempt']] as const) {
    assert.equal(overview.counts[plural].total, report.totals[plural]);
    assert.deepEqual(overview.counts[plural].outcomes, report.projection.counts.outcomes[singular]);
  }
  for (const series of overview.series) {
    const group = report.metric_groups.find(group => group.configuration_id === series.configuration_id &&
      group.stage === series.stage && group.outcome === series.outcome && group.name === series.name);
    assert.ok(group, `Missing shared series for ${series.name}/${series.stage}/${series.outcome}`);
    assert.equal(series.source, fixtureSource);
    assert.equal(series.numeric_count, group.numeric.count);
    if (group.numeric.mean === null) assert.equal(series.mean, null);
    else {
      assert.notEqual(series.mean, null);
      assert.ok(Math.abs(series.mean! - group.numeric.mean) <= Math.max(1, Math.abs(group.numeric.mean)) * 1e-12,
        `Go and TypeScript means agree within floating-point rounding: ${series.mean} vs ${group.numeric.mean}`);
    }
    assert.equal(series.unavailable_count, group.unavailable_count);
    assert.deepEqual(series.status_counts, Object.fromEntries(group.status_counts.map(({ value, count }) => [String(value), count])));
  }
  assert.equal(overview.series.length, report.metric_groups.length);
  const json = JSON.parse(reportToJSON(report)) as BenchmarkReport;
  assert.deepEqual(json.totals, report.totals);
  assert.equal(json.input.as_of, asOf);
  assert.equal(json.projection.records.length, records.length);
  const csvRows = reportToCSV(report).split('\r\n');
  for (const [kind, count] of [['raw_record', records.length], ['conversation', report.totals.conversations],
    ['turn', report.totals.turns], ['attempt', report.totals.attempts], ['metric_event', report.totals.metric_events]] as const) {
    assert.equal(csvRows.filter(row => row.startsWith(`"${kind}",`)).length, count, `CSV ${kind} rows`);
  }
  const markdown = reportToMarkdown(report);
  assert.ok(markdown.includes(`${report.totals.conversations} explicit conversations, ${report.totals.turns} turns, ${report.totals.attempts} attempts`));
  return report;
}

test('disposable Go API preserves phone SQLite observations and shared reports across HTTP', { skip: !target, timeout: 60_000 }, async t => {
  assert.ok(target);
  const server = new URL(target);
  assert.equal(server.protocol, 'http:');
  assert.equal(server.hostname, '127.0.0.1', 'Use the disposable loopback API runner.');
  assert.ok(server.port && server.port !== '8080', 'Never use the live collector on port 8080.');
  assert.equal(server.href, `${server.origin}/`, 'Supply only an origin without credentials, path, or query.');
  const request = (path: string, options: RequestInit = {}) => fetch(`${server.origin}/api/v1/${path}`, {
    ...options, redirect: 'error', signal: AbortSignal.timeout(5_000),
  });
  async function get<T>(path: string): Promise<T> {
    const response = await request(path);
    assert.equal(response.status, 200, await response.clone().text());
    return response.json() as Promise<T>;
  }
  // These checks all happen before writes. Existing metrics/experiments are also
  // forbidden even when the new canonical observation table happens to be empty.
  for (const path of ['phone/observations', 'phone/legacy-observations', 'phone/metrics', 'experiments']) {
    assert.equal((await get<{ total: number }>(path)).total, 0, `${path}: requires an empty disposable database`);
  }
  const db = new DatabaseSync(':memory:');
  try {
    let clock = 1_000;
    const installationID = `http-fixture-not-a-phone-${randomUUID()}`;
    const store = createBenchmarkStore(connection(db), { installationID, now: () => clock++ });
    store.setDevice({ installation_id: installationID, hardware: 'http-integration-fixture-not-a-phone', os_version: 'fixture', app_version: 'fixture', build: 'fixture' });
    const fixture = populate(store);
    const local = store.snapshot();
    assert.ok(local.length > 500, 'Exercise upload chunking and more than five default read pages.');
    const localProjection = projectObservations(local);
    assert.equal(localProjection.counts.conversations, 24);
    assert.equal(localProjection.counts.turns, 48);
    assert.equal(localProjection.counts.attempts, 96);
    assert.equal(localProjection.counts.metric_batches, 92);
    assert.equal(localProjection.counts.metric_events, 368);
    async function upload(records: PhoneObservation[], expected = 202) {
      const body = JSON.stringify({ schema_version: 1, device: store.device(), records });
      const response = await request('phone/observations', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body });
      assert.equal(response.status, expected, await response.clone().text());
      if (expected !== 202) return body;
      const ack = await response.json() as { status: string; record_count: number; accepted_record_ids: string[] };
      assert.equal(ack.status, 'accepted');
      assert.equal(ack.record_count, records.length);
      assert.deepEqual(ack.accepted_record_ids, records.map(record => record.record_id));
      store.acknowledge(ack.accepted_record_ids);
      return body;
    }
    // A terminal revision arrives before its registration and configuration.
    // Later older revisions must remain historical, never restart the entity.
    await upload([store.latest('conversation', fixture.conversations[0].id)!]);
    let retryPayload = '';
    while (store.stats().pending) {
      const pending = store.pending(50).map(row => JSON.parse(row.payload) as PhoneObservation);
      const payload = await upload(pending);
      retryPayload ||= payload;
    }
    assert.equal(store.stats().blocked, 0);
    assert.equal(store.snapshot().length, local.length, 'Acknowledgement keeps the durable archive.');
    const retried = await request('phone/observations', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: retryPayload });
    assert.equal(retried.status, 202, await retried.clone().text());
    assert.deepEqual((await retried.json() as { accepted_record_ids: string[] }).accepted_record_ids,
      (JSON.parse(retryPayload) as { records: PhoneObservation[] }).records.map(record => record.record_id));

    const first = await get<PhoneObservationPage>('phone/observations?limit=100');
    assert.equal(first.records.length, 100);
    assert.equal(first.total, local.length);
    assert.ok(first.next_cursor);
    const initialOverview = await get<Overview>(`phone/overview?as_of=${encodeURIComponent(first.as_of)}`);
    const lateID = store.registerConversation({ configuration_id: fixture.configurations[0] }, { source: 'manual', case_id: 'fixture-after-snapshot' });
    const late = store.latest('conversation', lateID)!;
    const original = local[0].record;
    await upload([{ ...original, timestamp_ms: original.timestamp_ms + 1 }], 409);
    await upload([{ ...original, record_id: randomUUID(), entity_id: randomUUID() }], 409);
    await upload([{ ...original, record_id: randomUUID(), record_sequence: 100_000 }], 409);
    await upload([late, { ...original, timestamp_ms: original.timestamp_ms + 1 }], 409);
    assert.equal((await get<PhoneObservationPage>('phone/observations')).total, local.length, 'A conflicting chunk inserts none of its records.');
    await upload([late]);
    assert.equal(store.stats().pending, 0);

    async function readAll(filters: ObservationFilters = {}, initial?: PhoneObservationPage) {
      const params = new URLSearchParams({ limit: '100', as_of: first.as_of });
      for (const [key, value] of Object.entries(filters)) params.set(key, String(value));
      let page = initial ?? await get<PhoneObservationPage>(`phone/observations?${params}`);
      const total = page.total;
      const records: StoredPhoneObservation[] = [];
      const selectedKeys: RecordReference[] = [];
      const cursors = new Set<string>();
      for (;;) {
        assert.equal(page.as_of, first.as_of);
        assert.equal(page.total, total);
        records.push(...page.records);
        assert.ok(Array.isArray(page.selected_record_keys), 'Each page identifies records selected by the filters.');
        selectedKeys.push(...page.selected_record_keys);
        if (!page.next_cursor) break;
        assert.ok(!cursors.has(page.next_cursor), 'Cursor must advance.');
        cursors.add(page.next_cursor);
        params.set('cursor', page.next_cursor);
        page = await get<PhoneObservationPage>(`phone/observations?${params}`);
      }
      assert.equal(records.length, total);
      assert.equal(new Set(records.map(({ record }) => record.record_id)).size, total);
      assert.ok(selectedKeys.every(key => records.some(stored => stored.device.installation_id === key.installation_id && stored.record.record_id === key.record_id)));
      return { records, selectedKeys };
    }
    const { records, selectedKeys } = await readAll({}, first);
    assert.deepEqual(new Set(records.map(({ record }) => record.record_id)), new Set(local.map(({ record }) => record.record_id)));
    const payloads = (items: StoredPhoneObservation[]) => new Map(items.map(({ device, record }) => [record.record_id, { device, record }]));
    assert.deepEqual(payloads(records), payloads(local), 'HTTP preserves every immutable record and its device attribution.');
    assert.ok(!records.some(({ record }) => record.entity_id === lateID));
    const stableOverview = await get<Overview>(`phone/overview?as_of=${encodeURIComponent(first.as_of)}`);
    assert.deepEqual(stableOverview, initialOverview, 'Overview and all export pages share one immutable snapshot.');
    const report = compareReport(stableOverview, records, first.as_of, {}, selectedKeys);
    assert.deepEqual(report.projection.counts, localProjection.counts);
    assert.equal(report.projection.coverage.unresolved_reference_count, 0);
    assert.equal(stableOverview.coverage.unresolved_record_count, 0);
    assert.deepEqual(stableOverview.counts.conversations.outcomes, { succeeded: 20, failed: 1, cancelled: 1, interrupted: 1, skipped: 1 });
    assert.equal(stableOverview.recent_conversations.length, 20);
    assert.equal(stableOverview.recent_runs.length, 20);
    assert.deepEqual(stableOverview.coverage.finalization, { complete: 115, partial: 5, unknown: 0, dropped_events: 4 });
    assert.equal(stableOverview.coverage.finalization.dropped_events, report.projection.coverage.dropped_events, 'Parent finalizations repeat the four attempt drops, not four additional losses.');
    const modelNames = new Set(stableOverview.series.flatMap(series => [series.configuration.asr.name, series.configuration.reasoning.name]));
    assert.deepEqual(modelNames, new Set(fixtureModels));
    for (const name of fixtureModels) assert.ok(reportToMarkdown(report).includes(name));
    const zero = stableOverview.series.find(series => series.name === 'cpu_percent');
    assert.equal(zero?.mean, 0);
    const unavailable = stableOverview.series.find(series => series.name === 'whole_device_power_watts');
    assert.equal(unavailable?.mean, null);
    assert.ok(unavailable && unavailable.unavailable_count > 0);
    const successfulDurations = report.duration_groups.filter(group => group.entity_kind === 'attempt' && group.outcome === 'succeeded');
    assert.equal(successfulDurations.length, 2);
    assert.ok(successfulDurations.every(group => group.numeric.p95_eligible_count === 40 && group.numeric.p95 !== null));

    const fresh = await get<PhoneObservationPage>('phone/observations');
    assert.equal(fresh.total, local.length + 1);
    assert.notEqual(fresh.as_of, first.as_of);
    assert.equal((await get<Overview>('phone/overview')).current_conversation?.entity_id, lateID);
    for (const suffix of [`&installation_id=${installationID}`, `&as_of=${encodeURIComponent(fresh.as_of)}`]) {
      const invalidCursor = await request(`phone/observations?cursor=${encodeURIComponent(first.next_cursor)}${suffix}`);
      assert.equal(invalidCursor.status, 400, 'Cursor binds both filters and snapshot.');
    }
    const firstConversationRecords = local.filter(({ record }) => record.entity_id === fixture.conversations[0].id || record.context.conversation_id === fixture.conversations[0].id);
    const filters: ObservationFilters[] = [
      { installation_id: installationID },
      { configuration_id: fixture.configurations[1] },
      { conversation_id: fixture.conversations[0].id },
      { source: 'acceptance' },
      { stage: 'reasoning', outcome: 'succeeded' },
      { since_ms: firstConversationRecords[0].record.timestamp_ms, until_ms: firstConversationRecords.at(-1)!.record.timestamp_ms },
      { installation_id: 'http-fixture-installation-not-present' },
    ];
    for (const filter of filters) {
      const { records: selected, selectedKeys: filterKeys } = await readAll(filter);
      const params = new URLSearchParams({ as_of: first.as_of });
      for (const [key, value] of Object.entries(filter)) params.set(key, String(value));
      const overview = await get<Overview>(`phone/overview?${params}`);
      const filteredReport = compareReport(overview, selected, first.as_of, filter, filterKeys);
      if (filter.configuration_id || filter.source) assert.equal(filteredReport.totals.conversations, 3);
      if (filter.conversation_id || filter.since_ms !== undefined) {
        assert.equal(filteredReport.totals.conversations, 1);
        assert.equal(filteredReport.totals.turns, 2);
        assert.equal(filteredReport.totals.attempts, 4);
        assert.equal(filteredReport.totals.metric_events, 16);
        assert.equal(filteredReport.projection.configurations.length, 1, 'Filtered exports include the referenced model configuration.');
      }
      if (filter.stage) assert.equal(filteredReport.totals.attempts, 40);
      if (filter.installation_id?.endsWith('not-present')) assert.equal(filteredReport.totals.canonical_records, 0);
    }
    assert.equal((await get<{ total: number }>(`phone/legacy-observations?as_of=${encodeURIComponent(first.as_of)}`)).total, 0);
    assert.equal((await get<{ total: number }>('experiments')).total, 0, 'Observation uploads never create benchmark execution jobs.');
    t.diagnostic(`Synthetic HTTP fixture only: ${local.length} snapshot records, 24 conversations, 48 turns, 96 attempts, 92 metric batches, 368 events; no inference or physical measurements.`);
  } finally {
    db.close();
  }
});
