import assert from 'node:assert/strict';
import test from 'node:test';
import { buildResultsPresentation, buildSweepComparison, createBenchmarkReport, projectObservations, reportToJSON } from '../index.ts';
import { presentationFixture } from './presentation-fixtures.ts';

test('derived processing uses each latest attempt once, includes failed retries, preserves zero and raw report', () => {
  const f = presentationFixture(); f.turn();
  f.attempt('asr-failed', 'transcription', 300, { status: 'failed' });
  f.attempt('asr-retry', 'transcription', 500);
  f.attempt('reply', 'reasoning', 0);
  f.attempt('loading', 'loading', 10000);
  f.add('attempt', 'asr-retry', { duration_ms: 700 }); f.finalize('asr-retry');
  const records = [...f.records, f.records[0]];
  const report = createBenchmarkReport(records, { as_of: 'test', generated_at_ms: 100 });
  const before = reportToJSON(report), result = buildResultsPresentation(report.projection);
  assert.equal(result.recordedProcessing.duration_ms, 1000);
  assert.equal(result.recordedProcessing.partial, false);
  assert.deepEqual(result.recordedProcessing.attempt_ids.sort(), ['asr-failed', 'asr-retry', 'reply']);
  assert.equal(result.turns[0].segments.find(segment => segment.attempt_id === 'reply')?.duration_ms, 0);
  assert.equal(reportToJSON(report), before);
  assert.equal(report.duration_groups.some(group => group.entity_kind === 'turn'), false);
});

test('missing, active and unfinalized attempts make a subtotal partial; zero is measured', () => {
  const f = presentationFixture(); f.turn(); f.attempt('asr', 'transcription', 0);
  let result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.recordedProcessing.duration_ms, 0); assert.equal(result.recordedProcessing.partial, true);
  assert.deepEqual(result.recordedProcessing.missing_stages, ['reasoning']);
  f.attempt('reply', 'reasoning', null, { status: 'running', complete: false });
  result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.recordedProcessing.duration_ms, 0); assert.equal(result.recordedProcessing.partial, true);
  assert.ok(result.recordedProcessing.reasons.some(reason => reason.includes('has not finished')));
  f.add('attempt', 'reply', { status: 'succeeded', duration_ms: 100 });
  assert.equal(buildResultsPresentation(projectObservations(f.records)).recordedProcessing.partial, true);
});

test('text and ASR-only modes have explicit stage expectations', () => {
  const f = presentationFixture(); f.turn('text', 'text'); f.attempt('reply', 'reasoning', 10, { turn: 'text' });
  assert.equal(buildResultsPresentation(projectObservations(f.records)).recordedProcessing.partial, false);
  f.add('sweep', 'sweep', { protocol: { mode: 'asr_only' } });
  f.conversation('asr-chat', {}, { sweep_id: 'sweep' }); f.turn('asr-turn', 'fixed_wav', { conversation_id: 'asr-chat' });
  f.attempt('asr', 'transcription', 20, { turn: 'asr-turn' });
  const result = buildResultsPresentation(projectObservations(f.records), { conversation_id: 'asr-chat' });
  assert.equal(result.recordedProcessing.duration_ms, 20); assert.equal(result.recordedProcessing.partial, false);
});

test('conversation lifecycle success does not hide cancelled child outcomes or missing stages', () => {
  const f = presentationFixture(); f.turn('cancelled', 'audio', {}, { status: 'cancelled' });
  f.attempt('asr', 'transcription', 400, { turn: 'cancelled', status: 'cancelled' });
  const result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.lifecycleStatuses[0].status, 'succeeded'); assert.deepEqual(result.outcomes, { cancelled: 1 });
  assert.equal(result.recordedProcessing.partial, true); assert.equal(result.typicalProcessingMs, null);
});

test('metric histories join compatible stages and outcomes but retain operation identities and gaps', () => {
  const f = presentationFixture(); f.turn();
  f.attempt('asr', 'transcription', 500, { status: 'failed', events: [f.metric('resident_memory_bytes', 1048576, 1000), f.metric('resident_memory_bytes', 2097152, 1500)] });
  f.attempt('reply', 'reasoning', 300, { events: [f.metric('resident_memory_bytes', 4194304, 1800)] });
  const result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.memory.length, 1); const memory = result.memory[0];
  assert.equal(memory.operationCount, 2); assert.equal(memory.sampleCount, 3); assert.equal(memory.displayUnit, 'MiB');
  assert.deepEqual(memory.stats, { count: 3, latest: 4, min: 1, max: 4, mean: 7 / 3 });
  assert.equal(memory.rawStats.max, 4194304); assert.equal(memory.unit, 'bytes');
  assert.deepEqual(memory.rawPoints.map(point => point.breakBefore), [true, false, true]);
  assert.deepEqual(memory.rawPoints.map(point => point.outcome), ['failed', 'failed', 'succeeded']);
  assert.equal(memory.rawPoints[2].context.attempt_id, 'reply'); assert.ok(memory.rawPoints[0].references.length);
});

test('metric grouping respects device, configuration, dataset, source, scope and units', () => {
  const f = presentationFixture(); f.turn('text', 'text');
  f.attempt('reply', 'reasoning', 20, { turn: 'text', events: [
    f.metric('cpu_percent', 125, 1000, { unit: 'percent_one_core', source: 'darwin.task_info' }),
    f.metric('cpu_percent', 10, 1500, { unit: 'percent', source: 'darwin.task_info' }),
    f.metric('cpu_percent', 11, 2000, { unit: 'percent', source: 'other' }),
    f.metric('cpu_percent', 12, 2500, { unit: 'percent', source: 'other', scope: 'device' }),
  ] });
  const result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.metrics.length, 4);
  const cpu = result.metrics.find(metric => metric.unit === 'percent_one_core')!;
  assert.equal(cpu.stats.max, 125); assert.equal(cpu.displayUnit, '%'); assert.match(cpu.description, /exceed 100%/);
  const other = f.records.map(record => ({ ...record, device: { ...record.device, installation_id: 'other' } }));
  assert.equal(buildResultsPresentation(projectObservations([...f.records, ...other])).metrics.length, 8);
});

test('thermal stays categorical, optional unavailable capability reasons remain separate, valid zero remains visible', () => {
  const f = presentationFixture(); f.turn('text', 'text');
  f.attempt('reply', 'reasoning', 0, { turn: 'text', events: [
    f.metric('thermal_state', 'nominal', 1000, { unit: 'state', scope: 'device' }),
    f.metric('thermal_state', null, 1500, { unit: 'state', scope: 'device', unavailable_reason: 'unknown' }),
    f.metric('thermal_state', 'serious', 2000, { unit: 'state', scope: 'device' }),
    f.metric('low_power_mode', false, 2000, { unit: 'state', scope: 'device' }),
    f.metric('power', null, 1000, { unit: 'watts', unavailable_reason: 'no_validated_power_provider' }),
    f.metric('resident_memory_bytes', 0, 1000),
  ] });
  const result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.thermal[0].kind, 'categorical'); assert.equal(result.thermal[0].stats.count, 0);
  assert.deepEqual(result.thermal[0].statusCounts, [{ value: 'nominal', count: 1 }, { value: 'serious', count: 1 }]);
  assert.deepEqual(result.thermal[0].points.map(point => point.breakBefore), [true, true, true]);
  assert.equal(result.metrics.find(metric => metric.name === 'low_power_mode')!.statusCounts[0].value, false);
  assert.equal(result.memory[0].stats.max, 0); assert.deepEqual(result.capabilities[0].reasons, ['no_validated_power_provider']);
});

test('downsampling keeps extrema, gap boundaries, at most 200 points and all-sample statistics', () => {
  const f = presentationFixture(); f.turn('text', 'text');
  const events = Array.from({ length: 1000 }, (_, i) => f.metric('resident_memory_bytes', i === 357 ? 999999 : i === 630 ? -1 : i, 1000 + i * 500 + (i > 499 ? 10000 : 0)));
  f.attempt('reply', 'reasoning', 100, { turn: 'text', events });
  const metric = buildResultsPresentation(projectObservations(f.records)).memory[0];
  assert.ok(metric.points.length <= 200); assert.equal(metric.rawPoints.length, 1000);
  assert.equal(metric.stats.count, 1000); assert.equal(metric.rawStats.max, 999999); assert.equal(metric.rawStats.min, -1);
  assert.ok(metric.points.some(point => point.value === 999999)); assert.ok(metric.points.some(point => point.value === -1));
  assert.ok(metric.points.some(point => point.value === 499)); assert.ok(metric.points.some(point => point.value === 500 && point.breakBefore));
  assert.equal(metric.rawStats.mean, events.reduce((sum, event) => sum + Number(event.value), 0) / 1000);
});

test('more than 200 operation boundaries never draw a line between retained executions', () => {
  const f = presentationFixture(); f.turn('text', 'text');
  for (let i = 0; i < 230; i++) f.attempt(`reply-${i}`, 'reasoning', 1, { turn: 'text', events: [f.metric('resident_memory_bytes', i, 1000 + i)] });
  const metric = buildResultsPresentation(projectObservations(f.records)).memory[0];
  assert.ok(metric.points.length <= 200); assert.ok(metric.omittedSegments > 0);
  assert.ok(metric.points.every(point => point.breakBefore)); assert.equal(metric.operationCount, 230);
});

test('scope narrows metrics and interval to the selected turn or attempt', () => {
  const f = presentationFixture(); f.turn();
  f.attempt('asr', 'transcription', 500, { events: [f.metric('resident_memory_bytes', 10, 1100)] });
  f.attempt('reply', 'reasoning', 1000, { events: [f.metric('resident_memory_bytes', 20, 5200)], data: { started_at_ms: 5000, finished_at_ms: 6000 } });
  const result = buildResultsPresentation(projectObservations(f.records), { attempt_id: 'reply' });
  assert.equal(result.metrics[0].sampleCount, 1); assert.equal(result.recordedProcessing.duration_ms, 1000);
  assert.equal(result.recordedProcessing.partial, false); assert.deepEqual(result.interval, { start_ms: 5000, end_ms: 6000 });
  assert.equal(result.metrics[0].points[0].elapsed_ms, 200);
});

test('absent expected turns and unknown input stage requirements keep derived totals partial', () => {
  const f = presentationFixture(); f.conversation('conversation', { expected_turns: 3 });
  f.turn('one', 'text'); f.attempt('reply', 'reasoning', 100, { turn: 'one' });
  let result = buildResultsPresentation(projectObservations(f.records));
  assert.equal(result.recordedProcessing.duration_ms, 100); assert.equal(result.recordedProcessing.partial, true);
  assert.ok(result.recordedProcessing.reasons.some(reason => reason.includes('2 expected turns')));
  f.turn('unknown', 'legacy-unknown'); f.attempt('unknown', 'reasoning', 50, { turn: 'unknown' });
  result = buildResultsPresentation(projectObservations(f.records), { turn_id: 'unknown' });
  assert.equal(result.recordedProcessing.partial, true);
  assert.ok(result.recordedProcessing.reasons.some(reason => reason.includes('Expected processing stages')));
});

test('typical processing requires two complete successful turns from one population', () => {
  const f = presentationFixture(); f.turn('one', 'text'); f.attempt('one', 'reasoning', 100, { turn: 'one' });
  assert.equal(buildResultsPresentation(projectObservations(f.records)).typicalProcessingMs, null);
  f.turn('two', 'text'); f.attempt('two', 'reasoning', 300, { turn: 'two' });
  assert.equal(buildResultsPresentation(projectObservations(f.records)).typicalProcessingMs, 200);
  f.configuration('other'); f.conversation('other-conversation', {}, { configuration_id: 'other' });
  f.turn('three', 'text', { conversation_id: 'other-conversation' }); f.attempt('three', 'reasoning', 500, { turn: 'three' });
  assert.equal(buildResultsPresentation(projectObservations(f.records)).typicalProcessingMs, null);
});

function sweepFixture() {
  const f = presentationFixture();
  f.add('sweep', 'sweep', { status: 'failed', planned_jobs: 3, protocol: { mode: 'fixed_script', conditions: { charging: 'unrecorded' } },
    protocol_revision: 'protocol-v1', dataset_revision: 'dataset-v1' });
  f.add('job', 'success', { status: 'succeeded', configuration_index: 0 }, { sweep_id: 'sweep', configuration_id: 'config' });
  f.add('job', 'failed', { status: 'failed', configuration_index: 1, reason_code: 'configuration_failed' }, { sweep_id: 'sweep' });
  f.add('job', 'skipped', { status: 'skipped', configuration_index: 1 }, { sweep_id: 'sweep' });
  f.conversation('conversation', { source: 'sweep' }, { sweep_id: 'sweep', job_id: 'success' });
  f.turn();
  return f;
}

test('sweep compares explicit planned job outcomes and retains configurations with zero conversations', () => {
  const f = sweepFixture(); f.attempt('reply', 'reasoning', 100);
  const result = buildSweepComparison(projectObservations(f.records), 'sweep');
  assert.equal(result.plannedJobs, 3); assert.deepEqual(result.outcomes, { succeeded: 1, failed: 1, skipped: 1 });
  assert.equal(result.configurations.length, 2);
  const failure = result.configurations.find(config => config.configuration_index === 1)!;
  assert.equal(failure.conversationCount, 0); assert.equal(failure.plannedJobs, 2); assert.equal(failure.stages.length, 0);
  assert.deepEqual(failure.outcomes, { failed: 1, skipped: 1 }); assert.match(failure.label, /Configuration 2/);
});

test('persisted plan identifies pre-configuration failures and unstarted jobs without inventing conversations', () => {
  const f = sweepFixture();
  const plan = { id: 'sweep', jobs: [
    { id: 'success', configuration_index: 0, status: 'succeeded' }, { id: 'failed', configuration_index: 1, status: 'failed' },
    { id: 'skipped', configuration_index: 1, status: 'skipped' }, { id: 'new', configuration_index: 2, status: 'planned' },
  ], configurations: [{ asr: { name: 'A' } }, { asr: { name: 'B', sha256: 'b' } }, { asr: { name: 'C' } }] };
  const result = buildSweepComparison(projectObservations(f.records), 'sweep', plan);
  assert.equal(result.plannedJobs, 4); assert.equal(result.outcomes.planned, 1);
  assert.equal(result.configurations[1].label, 'B'); assert.equal(result.configurations[2].label, 'C');
  assert.equal(result.configurations[2].conversationCount, 0);
});

test('sweep distributions use successful complete attempts, keep failures/counts and require 20 for p95', () => {
  const f = sweepFixture();
  for (let i = 0; i < 19; i++) f.attempt(`reply-${i}`, 'reasoning', i);
  f.attempt('failed', 'reasoning', 10000, { status: 'failed' });
  f.attempt('incomplete', 'reasoning', 20000, { complete: false });
  f.attempt('loading', 'loading', 1000); f.attempt('warmup', 'warmup', 2000);
  let result = buildSweepComparison(projectObservations(f.records), 'sweep');
  let reasoning = result.configurations[0].stages.find(stage => stage.stage === 'reasoning')!.groups[0];
  assert.equal(reasoning.stats.median, 9); assert.equal(reasoning.attemptCount, 21); assert.equal(reasoning.eligibleCount, 19);
  assert.equal(reasoning.stats.p95, null); assert.equal(result.configurations[0].stages.length, 3);
  f.attempt('reply-19', 'reasoning', 19);
  result = buildSweepComparison(projectObservations(f.records), 'sweep');
  reasoning = result.configurations[0].stages.find(stage => stage.stage === 'reasoning')!.groups[0];
  assert.equal(reasoning.stats.p95, 18); assert.equal(reasoning.stats.count, 20); assert.equal(reasoning.stats.median, 9.5);
});

test('sweep compatibility separates timing, device/build, mode, dataset and observed conditions', () => {
  const f = sweepFixture();
  f.attempt('a', 'reasoning', 10);
  f.attempt('timer', 'reasoning', 20, { data: { timer_boundary: 'other_timer' } });
  f.attempt('dataset', 'reasoning', 30, { data: { dataset_revision: 'dataset-v2' } });
  f.attempt('charging', 'reasoning', 40, { events: [f.metric('battery_state', 'charging', 1000, { unit: 'state', scope: 'device' })] });
  const a = f.records.find(item => item.record.kind === 'attempt' && item.record.entity_id === 'a')!;
  a.device = { ...a.device, build: 'other-build' };
  const result = buildSweepComparison(projectObservations(f.records), 'sweep');
  assert.equal(result.groups.length, 4); assert.equal(result.configurations[0].stages[0].groups.length, 4);
  assert.ok(result.limitations.some(message => message.includes('Do not pool')));
  assert.ok(result.groups.every(group => group.configurationKeys.length === 1));
});

test('different effective settings at the same planned matrix index keep distinct configuration rows', () => {
  const f = sweepFixture(); f.attempt('reply', 'reasoning', 10);
  f.configuration('variant', { settings: { threads: 8 } });
  f.add('job', 'variant-job', { status: 'succeeded', configuration_index: 0 }, { sweep_id: 'sweep', configuration_id: 'variant' });
  f.conversation('variant-conversation', { source: 'sweep' }, { configuration_id: 'variant', sweep_id: 'sweep', job_id: 'variant-job' });
  f.turn('variant-turn', 'text', { conversation_id: 'variant-conversation' });
  f.attempt('variant-reply', 'reasoning', 20, { turn: 'variant-turn' });
  const result = buildSweepComparison(projectObservations(f.records), 'sweep');
  const variants = result.configurations.filter(config => config.configuration_index === 0);
  assert.equal(variants.length, 2); assert.equal(variants[0].stages[0].groups[0].stats.count, 1);
  assert.equal(variants[1].stages[0].groups[0].stats.count, 1);
  assert.notEqual(variants[0].key, variants[1].key);
});
