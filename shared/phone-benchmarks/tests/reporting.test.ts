import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createBenchmarkReport, ObservationConflictError, projectLegacyObservations, projectObservations,
  reportToCSV, reportToJSON, reportToMarkdown, summarizeNumbers,
} from '../index.ts';
import type { PhoneDevice, PhoneMetricBatch, PhoneObservation, StoredPhoneObservation } from '../index.ts';

// Deterministic synthetic fixtures only. These values are not phone measurements.
const device: PhoneDevice = { installation_id: 'fixture-installation-a', hardware: 'fixture-phone', os_version: 'fixture-os', app_version: 'fixture-app', build: 'fixture-build' };
function fixture() {
  let sequence = 0;
  const records: StoredPhoneObservation[] = [];
  const add = (kind: PhoneObservation['kind'], id: string, data: Record<string, unknown>, context: PhoneObservation['context'] = {}, revision = 0) => {
    const record: PhoneObservation = { schema_version: 1, record_id: `fixture-record-${sequence}`, producer_stream_id: 'fixture-stream',
      record_sequence: sequence++, timestamp_ms: 1000 + sequence, kind, entity_id: id, revision, context, data };
    const stored = { device: { ...device }, record, received_at_ms: 2000 + sequence }; records.push(stored); return stored;
  };
  add('configuration', 'fixture-config', { asr: { id: 'fixture-asr', name: 'Fixture ASR', sha256: 'fixture-asr-hash' },
    reasoning: { id: 'fixture-reasoning', name: 'Fixture Reasoning', sha256: 'fixture-reasoning-hash' }, settings: { threads: 2 }, runtime: { revision: 'fixture-runtime' } });
  add('conversation', 'fixture-conversation', { status: 'active', source: 'manual', dataset_revision: 'fixture-dataset', protocol_revision: 'fixture-protocol' }, { configuration_id: 'fixture-config' });
  const attempt = (id: string, value: number | null, options: { status?: string; complete?: boolean; stage?: string; drop?: number; reason?: string } = {}) => {
    const turn = `turn-${id}`;
    add('turn', turn, { status: options.status ?? 'succeeded', turn_index: sequence, duration_ms: value }, { conversation_id: 'fixture-conversation' });
    add('attempt', id, { status: options.status ?? 'succeeded', stage: options.stage ?? 'reasoning', native_run_id: 'reused-native-id', duration_ms: value }, { turn_id: turn });
    const batch: PhoneMetricBatch = { schema_version: 1, run_id: 'reused-native-id', sequence_number: 0, timestamp_ms: 1000,
      events: [{ schema_version: 1, run_id: 'reused-native-id', sequence: 0, timestamp_ms: 1000, name: 'duration', value, unit: 'milliseconds', scope: 'run', source: 'fixture-native', ...(value === null ? { unavailable_reason: options.reason ?? 'fixture unavailable' } : {}) }] };
    add('metric_batch', `batch-${id}`, { batch }, { attempt_id: id });
    const required = records.filter(stored => stored.record.kind === 'attempt' && stored.record.entity_id === id || stored.record.context.attempt_id === id).map(stored => stored.record.record_id);
    if (options.complete !== false) add('finalization', `final-${id}`, { status: 'complete', event_count: 1, dropped_events: options.drop ?? 0,
      target_revision: 0, required_record_ids: required, record_count: required.length }, { attempt_id: id });
  };
  return { records, add, attempt };
}

test('out-of-order records resolve explicit parents and retain model names and hashes', () => {
  const f = fixture(); f.attempt('a', 0);
  const p = projectObservations([...f.records].reverse());
  assert.equal(p.conversations[0].turns[0].attempts[0].entity_id, 'a');
  assert.equal(p.attempts[0].context.configuration_id, 'fixture-config');
  assert.equal(p.configurations[0].asr?.name, 'Fixture ASR');
  assert.equal(p.configurations[0].reasoning?.sha256, 'fixture-reasoning-hash');
  assert.equal(p.metric_groups[0].numeric.mean, 0);
  assert.equal(p.metric_groups[0].dataset_revision, 'fixture-dataset');
  assert.equal(p.unresolved_references.length, 0);
});

test('identical retries deduplicate, each immutable identity collision rejects, installations remain distinct', () => {
  const f = fixture(); f.attempt('a', 1);
  assert.equal(projectObservations([...f.records, f.records[0]]).counts.records, f.records.length);
  for (const mutation of [
    (r: PhoneObservation) => { r.data = { different: true }; },
    (r: PhoneObservation) => { r.record_id = 'different-id'; },
    (r: PhoneObservation) => { r.record_id = 'different-id'; r.record_sequence = 99; },
  ]) {
    const collision = JSON.parse(JSON.stringify(f.records[0])) as StoredPhoneObservation; mutation(collision.record);
    assert.throws(() => projectObservations([...f.records, collision]), ObservationConflictError);
  }
  const other = f.records.map(record => ({ ...record, device: { ...device, installation_id: 'fixture-installation-b' } }));
  const p = projectObservations([...f.records, ...other]);
  assert.equal(p.counts.conversations, 2); assert.equal(p.counts.attempts, 2); assert.equal(p.metric_groups.length, 2);
});

test('late lower revisions do not resurrect terminal work, missing parents remain unresolved', () => {
  const f = fixture(); f.attempt('a', 1);
  f.add('conversation', 'fixture-conversation', { status: 'interrupted' }, { configuration_id: 'fixture-config' }, 2);
  f.add('conversation', 'fixture-conversation', { status: 'running' }, { configuration_id: 'fixture-config' }, 1);
  assert.equal(projectObservations(f.records).conversations[0].status, 'interrupted');
  const orphan = f.records.filter(record => record.record.kind === 'metric_batch');
  const p = projectObservations(orphan);
  assert.equal(p.counts.conversations, 0); assert.equal(p.raw_metrics[0].outcome, null);
  assert.equal(p.unresolved_references[0].field, 'attempt_id'); assert.equal(p.coverage.completeness, 'partial');
});

test('complete markers do not hide drops or missing stream records', () => {
  const f = fixture(); f.attempt('a', 10, { drop: 1 });
  f.add('finalization', 'conversation-final', { status: 'complete', event_count: 1, dropped_events: 0 }, { conversation_id: 'fixture-conversation' });
  const p = projectObservations(f.records);
  assert.equal(p.attempts[0].status, 'succeeded'); assert.equal(p.attempts[0].completeness, 'partial');
  assert.equal(p.conversations[0].completeness, 'partial'); assert.equal(p.metric_groups[0].numeric.p95_eligible_count, 0);
  const g = fixture(); g.attempt('a', 10);
  const missing = g.records.filter(record => record.record.kind !== 'metric_batch');
  assert.equal(projectObservations(missing).attempts[0].completeness, 'partial');
  assert.equal(projectObservations(missing).coverage.sequence_gaps.length, 1);
});

test('p95 needs 20 eligible samples in each compatible group and never averages means', () => {
  const f = fixture();
  for (let i = 0; i < 19; i++) f.attempt(`success-${i}`, i);
  f.attempt('failed', 1000, { status: 'failed' }); f.attempt('unfinalized', 1000, { complete: false });
  let p = projectObservations(f.records);
  let succeeded = p.metric_groups.find(group => group.outcome === 'succeeded')!;
  assert.equal(succeeded.numeric.p95, null); assert.equal(succeeded.numeric.p95_eligible_count, 0);
  let duration = createBenchmarkReport(f.records, { as_of: 'fixture-p95', generated_at_ms: 3000 }).duration_groups.find(g => g.entity_kind === 'attempt' && g.outcome === 'succeeded')!;
  assert.equal(duration.numeric.p95, null); assert.equal(duration.numeric.p95_eligible_count, 19);
  f.attempt('success-19', 19); p = projectObservations(f.records);
  succeeded = p.metric_groups.find(group => group.outcome === 'succeeded')!;
  assert.equal(succeeded.numeric.p95, null); assert.equal(succeeded.numeric.p95_eligible_count, 0);
  duration = createBenchmarkReport(f.records, { as_of: 'fixture-p95', generated_at_ms: 3000 }).duration_groups.find(g => g.entity_kind === 'attempt' && g.outcome === 'succeeded')!;
  assert.equal(duration.numeric.p95, 18); assert.equal(duration.numeric.p95_eligible_count, 20);
  assert.equal(succeeded.numeric.mean, (190 + 1000) / 21);
  assert.equal(p.metric_groups.find(group => group.outcome === 'failed')!.numeric.p95, null);
  assert.equal(summarizeNumbers([0, 0, 0, 100]).mean, 25);
});

test('all numeric, null and status metric families retain original source, scope and unit', () => {
  const f = fixture(); f.attempt('a', null, { reason: 'no sensor' });
  const batchRecord = f.records.find(record => record.record.kind === 'metric_batch')!;
  const batch = batchRecord.record.data.batch as PhoneMetricBatch;
  batch.events.push(
    { ...batch.events[0], sequence: 1, value: 'nominal', name: 'thermal', unit: 'status' },
    { ...batch.events[0], sequence: 2, value: false, name: 'low_power_mode', unit: 'status' },
    { ...batch.events[0], sequence: 3, value: 0, name: 'power', unit: 'watts', scope: 'process', unavailable_reason: undefined },
    { ...batch.events[0], sequence: 4, value: null, name: 'power', unit: 'watts', scope: 'device', unavailable_reason: 'not measured' },
  );
  const p = projectObservations(f.records);
  assert.equal(p.metric_groups.length, 5);
  assert.equal(p.metric_groups.find(g => g.name === 'thermal')!.status_counts[0].value, 'nominal');
  assert.equal(p.metric_groups.find(g => g.name === 'low_power_mode')!.status_counts[0].value, false);
  assert.equal(p.metric_groups.find(g => g.scope === 'process')!.numeric.mean, 0);
  assert.equal(p.metric_groups.find(g => g.scope === 'device')!.unavailable_reasons[0].reason, 'not measured');
});

test('reports are immutable snapshots with filters, all formats, raw references and safe CSV strings', () => {
  const f = fixture(); f.attempt('a', 0);
  (f.records[0].record.data.asr as Record<string, unknown>).name = '=HYPERLINK("fixture")';
  const batch = f.records.find(record => record.record.kind === 'metric_batch')!.record.data.batch as PhoneMetricBatch;
  batch.events[0].name = '=HYPERLINK("fixture")';
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-watermark', generated_at_ms: 3000, filters: { conversation_id: 'fixture-conversation' } });
  assert.ok(Object.isFrozen(report)); assert.ok(Object.isFrozen(report.projection.records[0].record));
  const json = reportToJSON(report); const csv = reportToCSV(report); const markdown = reportToMarkdown(report);
  f.records[0].record.data.asr = null;
  assert.equal(reportToJSON(report), json); assert.equal(report.projection.configurations.length, 1);
  assert.ok(csv.includes('"\'=HYPERLINK(""fixture"")"')); assert.ok(csv.includes('"0","number"'));
  assert.ok(markdown.includes('fixture-watermark')); assert.ok(markdown.includes('Fixture Reasoning'));
  assert.equal(JSON.parse(json).input.filters.conversation_id, 'fixture-conversation');
  assert.ok(report.metric_groups[0].event_references[0].record_id);
});

test('over 100 batches use the entire snapshot and do not change with presentation pagination', () => {
  const f = fixture(); for (let i = 0; i < 125; i++) f.attempt(`fixture-${i}`, i);
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-125', generated_at_ms: 3000 });
  assert.equal(report.totals.metric_batches, 125); assert.equal(report.totals.attempts, 125);
  assert.equal(report.metric_groups[0].numeric.count, 125); assert.equal(report.metric_groups[0].numeric.mean, 62);
  assert.equal(report.duration_groups.find(group => group.entity_kind === 'attempt')!.numeric.count, 125);
});

test('normalized JSON stays bounded and exactly reconstructs expanded metrics and hierarchy', () => {
  const f = fixture();
  (f.records[0].record.data.runtime as Record<string, unknown>).build_options = 'fixture-metadata-'.repeat(150);
  for (let i = 0; i < 125; i++) f.attempt(`export-${i}`, i);
  const report = createBenchmarkReport(f.records, { as_of: 'export-fixture', generated_at_ms: 3000, filters: { conversation_id: 'fixture-conversation' } });
  const encoded = reportToJSON(report), exported = JSON.parse(encoded);
  assert.equal(exported.report_version, 2);
  assert.equal(exported.encoding, 'canonical_records_with_flat_summaries_v2');
  assert.equal(exported.projection.raw_metrics, undefined);
  assert.equal(exported.projection.conversations[0].turns, undefined);
  assert.equal(exported.projection.conversations[0].turn_ids.length, 125);
  assert.ok(encoded.length < JSON.stringify(report.projection.records).length * 15, 'Expanded metadata must not grow the file by orders of magnitude');
  const restored = createBenchmarkReport(exported.projection.records, { generated_at_ms: exported.generated_at_ms,
    as_of: exported.input.as_of, filters: exported.input.filters, input_already_filtered: exported.input.filtered,
    selected_record_keys: exported.input.selected_record_keys, legacy_records: exported.legacy.records });
  assert.deepEqual(restored.projection, report.projection);
  assert.deepEqual(restored.duration_groups, report.duration_groups);
  assert.deepEqual(restored.metric_groups, report.metric_groups);
  assert.deepEqual(restored.totals, report.totals);
});

test('legacy preserves unknown conversations, explicit installation-scoped links, zero and unavailable', () => {
  const f = fixture(); f.attempt('a', 0);
  const batch = f.records.find(record => record.record.kind === 'metric_batch')!.record.data.batch as PhoneMetricBatch;
  const legacy = { device, batch: { ...batch, experiment_id: 'legacy-exp' }, received_at_ms: 2000 };
  const p = projectLegacyObservations([legacy, legacy, { ...legacy, device: { ...device, installation_id: 'fixture-b' } }]);
  assert.equal(p.batch_count, 2); assert.equal(p.duplicate_batches, 1); assert.equal(p.conversation_count, null);
  assert.equal(p.metric_groups.length, 2); assert.equal(p.raw_metrics[0].outcome, null);
  assert.equal(p.raw_metrics[0].context.conversation_id, undefined);
  assert.throws(() => projectLegacyObservations([legacy, { ...legacy, batch: { ...legacy.batch, timestamp_ms: 1 } }]), ObservationConflictError);
});

test('bare complete markers, wrong target revisions and missing required records remain partial', () => {
  for (const mutate of [
    (data: Record<string, unknown>) => { delete data.required_record_ids; },
    (data: Record<string, unknown>) => { data.target_revision = 99; },
    (data: Record<string, unknown>) => { data.required_record_ids = ['not-received']; data.record_count = 1; },
    (data: Record<string, unknown>) => { data.record_count = 999; },
    (data: Record<string, unknown>) => { delete data.dropped_events; },
  ]) {
    const f = fixture(); f.attempt('a', 10); mutate(f.records.find(r => r.record.kind === 'finalization')!.record.data);
    assert.equal(projectObservations(f.records).attempts[0].completeness, 'partial');
  }
});

test('strict finalization can reference known parent manifests and deduplicates native batch retries', () => {
  const f = fixture(); f.attempt('a', 10);
  const original = f.records.find(record => record.record.kind === 'metric_batch')!;
  const duplicate = f.add('metric_batch', 'duplicate-native-batch', original.record.data, original.record.context);
  const finalization = f.records.find(record => record.record.kind === 'finalization')!;
  const required = [...finalization.record.data.required_record_ids as string[], duplicate.record.record_id,
    ...f.records.filter(record => ['configuration', 'conversation', 'turn'].includes(record.record.kind)).map(record => record.record.record_id)];
  finalization.record.data.required_record_ids = required; finalization.record.data.record_count = required.length;
  const p = projectObservations(f.records);
  assert.equal(p.attempts[0].completeness, 'complete'); assert.equal(p.counts.metric_batches, 1); assert.equal(p.counts.metric_events, 1);
  assert.equal(p.raw_metrics[0].references.length, 2);
});

test('filtered exports retain support parents without invented stream gaps or parent duration samples', () => {
  const f = fixture(); f.attempt('asr', 10, { stage: 'transcription' }); f.attempt('reply', 20);
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-stage', generated_at_ms: 3000, filters: { stage: 'reasoning' } });
  assert.equal(report.projection.counts.conversations, 1); assert.equal(report.projection.counts.turns, 1);
  assert.equal(report.projection.coverage.sequence_gaps.length, 0);
  assert.equal(report.projection.attempts[0].completeness, 'complete');
  assert.equal(report.duration_groups.length, 1); assert.equal(report.duration_groups[0].entity_kind, 'attempt');
  assert.equal(report.duration_groups[0].numeric.mean, 20);
  const serverReport = createBenchmarkReport(report.projection.records, { as_of: 'fixture-server-stage', generated_at_ms: 3000,
    filters: { stage: 'reasoning' }, input_already_filtered: true, selected_record_keys: report.input.selected_record_keys });
  assert.equal(serverReport.projection.coverage.sequence_gaps.length, 0);
  assert.deepEqual(serverReport.duration_groups, report.duration_groups);
});

test('canonical and legacy normalized transport copies count once, conflicts reject, separate attempts remain distinct', () => {
  const f = fixture(); f.attempt('a', 0); f.attempt('b', 0);
  const batch = f.records.find(record => record.record.kind === 'metric_batch')!.record.data.batch as PhoneMetricBatch;
  const legacy = { device, batch: { ...batch, experiment_id: 'server-exp', events: batch.events.map(event => ({ ...event, experiment_id: 'server-exp' })) }, received_at_ms: 2000 };
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-dedupe', generated_at_ms: 3000, legacy_records: [legacy] });
  assert.equal(report.totals.metric_batches, 2); assert.equal(report.totals.metric_events, 2);
  assert.equal(report.legacy.batch_count, 0); assert.equal(report.legacy.canonical_duplicate_batches, 1);
  assert.throws(() => createBenchmarkReport(f.records, { as_of: 'fixture-conflict', generated_at_ms: 3000,
    legacy_records: [{ ...legacy, batch: { ...legacy.batch, timestamp_ms: 7 } }] }), ObservationConflictError);
});

test('raw resource and repeated timing events do not produce execution p95', () => {
  const f = fixture(); f.attempt('a', 10);
  const batch = f.records.find(r => r.record.kind === 'metric_batch')!.record.data.batch as PhoneMetricBatch;
  batch.events = Array.from({ length: 50 }, (_, sequence) => ({ ...batch.events[0], sequence, value: sequence }));
  f.records.find(r => r.record.kind === 'finalization')!.record.data.event_count = 50;
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-polling', generated_at_ms: 3000 });
  assert.equal(report.metric_groups[0].numeric.count, 50); assert.equal(report.metric_groups[0].numeric.p95, null);
  const durations = report.duration_groups.find(g => g.entity_kind === 'attempt')!;
  assert.equal(durations.numeric.count, 1); assert.equal(durations.numeric.p95_eligible_count, 1); assert.equal(durations.numeric.p95, null);
});

test('legacy explicit native metadata retains stage without inventing turn IDs, ambiguous run lifetimes stay unknown', () => {
  const f = fixture(); f.attempt('a', 0);
  const native = f.records.find(record => record.record.kind === 'metric_batch')!.record.data.batch as PhoneMetricBatch;
  const metadata: PhoneMetricBatch = { ...native, run_id: 'legacy-metadata', events: [
    { ...native.events[0], run_id: 'legacy-metadata', name: 'native_run_id', value: native.run_id, unit: 'status' },
    { ...native.events[0], run_id: 'legacy-metadata', sequence: 1, name: 'model_purpose', value: 'transcription', unit: 'status' },
  ] };
  const records = [{ device, batch: native, received_at_ms: 1000 }, { device, batch: metadata, received_at_ms: 1000 }];
  let p = projectLegacyObservations(records);
  assert.equal(p.raw_metrics.find(metric => metric.event.name === 'duration')!.stage, 'transcription');
  assert.equal(p.raw_metrics[0].context.turn_id, undefined);
  p = projectLegacyObservations([...records, { device, batch: { ...metadata, run_id: 'metadata-other-lifetime',
    events: metadata.events.map(event => event.name === 'model_purpose' ? { ...event, value: 'reasoning' } : event) }, received_at_ms: 2000 }]);
  assert.equal(p.raw_metrics.find(metric => metric.event.name === 'duration')!.stage, null);
});

test('builds and dataset revisions separate duration populations below p95 minimum', () => {
  const f = fixture();
  for (let i = 0; i < 30; i++) {
    f.attempt(`a-${i}`, i);
    const attempt = f.records.find(record => record.record.kind === 'attempt' && record.record.entity_id === `a-${i}`)!;
    if (i < 10) attempt.device = { ...attempt.device, build: 'fixture-build-other' };
    else if (i < 20) attempt.record.data.dataset_revision = 'fixture-dataset-other';
  }
  const report = createBenchmarkReport(f.records, { as_of: 'fixture-compatible', generated_at_ms: 3000 });
  const groups = report.duration_groups.filter(group => group.entity_kind === 'attempt');
  assert.equal(groups.length, 3); assert.ok(groups.every(group => group.numeric.count === 10 && group.numeric.p95 === null));
  assert.equal(report.metric_groups.length, 3);
});

test('parent finalization drop counters do not duplicate child drops', () => {
  const f = fixture();
  f.add('sweep', 'fixture-sweep', { status: 'succeeded' });
  f.records.find(record => record.record.kind === 'conversation')!.record.context.sweep_id = 'fixture-sweep';
  f.attempt('a', 10, { drop: 3 });
  f.add('finalization', 'conversation-final', { status: 'partial', dropped_events: 3, event_count: 1 }, { conversation_id: 'fixture-conversation' });
  f.add('finalization', 'sweep-final', { status: 'partial', dropped_events: 3, event_count: 1 }, { sweep_id: 'fixture-sweep' });
  assert.equal(projectObservations(f.records).coverage.dropped_events, 3);
  f.records.find(record => record.record.entity_id === 'sweep-final')!.record.data.dropped_events = 5;
  assert.equal(projectObservations(f.records).coverage.dropped_events, 5);
});
