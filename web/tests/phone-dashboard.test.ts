import assert from 'node:assert/strict'
import test from 'node:test'
import { createCoalescedRefresh, metricSeriesKey, numericSegments, phoneQuery, parsePhoneFilters, type LegacyPhoneObservation, type PhoneMetricSeries } from '../src/lib/phoneBenchmarkTypes.ts'
import { legacyRunRecords, projectedSeries } from '../src/lib/phoneProjection.ts'
import { createBenchmarkReport, projectLegacyObservations } from '../../shared/phone-benchmarks/index.ts'
import { getPhoneSnapshot } from '../src/lib/phoneBenchmarkApi.ts'

test('overview and detail preserve sweep and conversation scope through URL filters', () => {
  const selected = { sweep_id: 'sweep-a', conversation_id: 'conversation-b', installation_id: 'phone-c', stage: 'reasoning', since_ms: 100 }
  const params = new URLSearchParams(phoneQuery(selected, { return: 'ignored-navigation-value' }))
  assert.deepEqual(parsePhoneFilters(params), selected)
  params.set('until_ms', 'not-a-number')
  assert.deepEqual(parsePhoneFilters(params), selected)
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}
function batch(installation: string, run: string, values: Record<string, string | number | null>): LegacyPhoneObservation {
  return { device: { installation_id: installation, hardware: 'iPhone', os_version: '18', app_version: '1', build: '2' }, received_at_ms: 100,
    batch: { schema_version: 1, experiment_id: `${installation}:${run}`, run_id: run, timestamp_ms: 99, sequence_number: 0,
      events: Object.entries(values).map(([name, value], sequence) => ({ schema_version: 1, run_id: run, sequence, timestamp_ms: 99, name, value, unit: typeof value === 'string' ? 'status' : 'milliseconds', scope: 'run', source: 'test.native', ...(value === null ? { unavailable_reason: 'No provider' } : {}) })) } }
}

test('event bursts coalesce into one follow-up and requests never overlap', async () => {
  const first = deferred<number>(), second = deferred<number>()
  let loads = 0
  const applied: number[] = []
  const refresh = createCoalescedRefresh(() => (++loads === 1 ? first.promise : second.promise), value => applied.push(value), error => { throw error })
  const pending = refresh.refresh()
  void refresh.refresh(); void refresh.refresh(); void refresh.refresh()
  assert.equal(loads, 1)
  first.resolve(1)
  await Promise.resolve(); await Promise.resolve()
  assert.equal(loads, 2)
  second.resolve(2)
  await pending
  assert.deepEqual(applied, [1, 2])
})

test('disposed filter refresh cannot replace a newer filter snapshot', async () => {
  const stale = deferred<string>()
  const applied: string[] = []
  let oldSignal: AbortSignal | undefined
  const old = createCoalescedRefresh(signal => { oldSignal = signal; return stale.promise }, result => applied.push(result), () => {})
  const pending = old.refresh()
  old.dispose()
  await createCoalescedRefresh(async () => 'selected-device', result => applied.push(result), () => {}).refresh()
  stale.resolve('old-device')
  await pending
  assert.equal(oldSignal?.aborted, true)
  assert.deepEqual(applied, ['selected-device'])
})

test('numeric charts retain true zero and break lines across unavailable readings', () => {
  assert.deepEqual(numericSegments([{ timestamp: 3, value: 5 }, { timestamp: 1, value: 0 }, { timestamp: 2, value: null }, { timestamp: 4, value: Number.NaN }]), [[{ timestamp: 1, value: 0 }], [{ timestamp: 3, value: 5 }]])
})

test('filters and snapshot watermark preserve date zero and encode identities', () => {
  const params = new URLSearchParams(phoneQuery({ installation_id: 'a+b & c', since_ms: 0, source: 'manual', outcome: 'failed' }, { as_of: 'opaque:snapshot', cursor: 'cursor/2', limit: 100 }))
  assert.equal(params.get('installation_id'), 'a+b & c')
  assert.equal(params.get('since_ms'), '0')
  assert.equal(params.get('as_of'), 'opaque:snapshot')
  assert.equal(params.get('outcome'), 'failed')
})

test('legacy native and explicit parent links stay within installation and preserve missing lifecycle', () => {
  const a = batch('phone-a', 'native-1', { duration_ms: 0 })
  const metadata = batch('phone-a', 'metadata-1', { native_run_id: 'native-1', model_purpose: 'transcription', model_name: 'Whisper', model_sha256: 'aaa' })
  const reply = batch('phone-a', 'reply-1', { parent_run_id: 'native-1', model_purpose: 'reasoning', duration_ms: 10 })
  const b = batch('phone-b', 'native-1', { duration_ms: 900 })
  const linked = legacyRunRecords([a, metadata, reply, b], 'native-1', 'phone-a')
  assert.equal(linked.length, 3)
  assert.ok(linked.every(item => item.device?.installation_id === 'phone-a'))
  const projection = projectLegacyObservations(linked)
  assert.equal(projection.conversation_count, null)
  assert.ok(projection.raw_metrics.every(metric => metric.outcome === null))
  assert.ok(projection.raw_metrics.every(metric => metric.context.conversation_id === undefined))
})

test('missing ASR parent remains a partial unlinked run and never a typed-turn guess', () => {
  const reply = batch('phone-a', 'reply-1', { parent_run_id: 'missing-native', model_purpose: 'reasoning', turn_id: 'explicit-turn' })
  const linked = legacyRunRecords([reply], 'reply-1', 'phone-a')
  const report = createBenchmarkReport([], { as_of: 'one', generated_at_ms: 100, legacy_records: linked })
  assert.equal(report.totals.conversations, 0)
  assert.equal(report.legacy.conversation_count, null)
  assert.ok(report.legacy.raw_metrics.every(metric => metric.outcome === null))
})

test('ambiguous restarted native counters cannot pull two turns into one history', () => {
  const raw = batch('phone-a', 'native-1', { duration_ms: 1 })
  const first = batch('phone-a', 'metadata-first', { native_run_id: 'native-1', turn_id: 'turn-first' })
  const second = batch('phone-a', 'metadata-second', { native_run_id: 'native-1', turn_id: 'turn-second' })
  assert.deepEqual(legacyRunRecords([raw, first, second], 'native-1', 'phone-a'), [raw])
})

test('unknown installations keep legacy experiment containers isolated', () => {
  const first = { ...batch('first', 'same-native', { duration_ms: 1 }), device: null }
  const second = { ...batch('second', 'same-native', { duration_ms: 100 }), device: null }
  assert.deepEqual(legacyRunRecords([first, second], 'same-native', null), [])
  assert.deepEqual(legacyRunRecords([first, second], 'same-native', null, first.batch.experiment_id), [first])
})

test('projection displays status samples and unavailable values without merging scope or unit', () => {
  const projection = projectLegacyObservations([batch('a', 'run-a', { duration_ms: 0, thermal_state: 'nominal', energy: null })])
  const result = projectedSeries(projection.metric_groups, { raw_metrics: projection.raw_metrics, configurations: [] })
  assert.equal(result.series.find(item => item.name === 'duration_ms')?.mean, 0)
  assert.equal(result.series.find(item => item.name === 'thermal_state')?.status_counts.nominal, 1)
  assert.equal(result.series.find(item => item.name === 'energy')?.unavailable_count, 1)
  const base = result.series[0]
  assert.notEqual(metricSeriesKey(base), metricSeriesKey({ ...base, scope: 'device' }))
  assert.notEqual(metricSeriesKey(base), metricSeriesKey({ ...base, source: 'other' }))
  assert.notEqual(metricSeriesKey(base), metricSeriesKey({ ...base, configuration_id: 'another' }))
  assert.notEqual(metricSeriesKey(base), metricSeriesKey({ ...base, unit: 'seconds' } as PhoneMetricSeries))
})

test('report and displayed numeric totals include more than the visible activity page', () => {
  const records = Array.from({ length: 125 }, (_, i) => batch('a', `run-${i}`, { duration_ms: i }))
  const report = createBenchmarkReport([], { as_of: 'full-snapshot', generated_at_ms: 100, legacy_records: records })
  const displayed = projectedSeries(report.legacy.metric_groups, { raw_metrics: report.legacy.raw_metrics, configurations: [] })
  assert.equal(report.totals.legacy_batches, 125)
  assert.equal(displayed.series[0].numeric_count, 125)
  assert.equal(displayed.series[0].mean, 62)
  assert.equal(displayed.samples.values().next().value?.length, 125)
})

test('snapshot loader exhausts cursor pages with the same watermark and all selected keys', async () => {
  const originalFetch = globalThis.fetch
  const calls: URL[] = []
  const records = Array.from({ length: 125 }, (_, index) => ({ device: { installation_id: 'a' }, record: { record_id: `record-${index}` } }))
  globalThis.fetch = async input => {
    const url = new URL(String(input), 'http://fixture.invalid')
    calls.push(url)
    assert.equal(url.searchParams.get('as_of'), 'pinned')
    assert.equal(url.searchParams.get('installation_id'), 'a')
    if (url.pathname.endsWith('/legacy-observations')) return Response.json({ schema_version: 1, as_of: 'pinned', batches: [], total: 0, next_cursor: null })
    const next = url.searchParams.has('cursor')
    const page = records.slice(next ? 100 : 0, next ? 125 : 100)
    return Response.json({ schema_version: 1, as_of: 'pinned', records: page, total: 125, next_cursor: next ? null : 'second', selected_record_keys: page.map(item => ({ installation_id: 'a', record_id: item.record.record_id })), filtered: true })
  }
  try {
    const snapshot = await getPhoneSnapshot({ installation_id: 'a' }, 'pinned')
    assert.equal(snapshot.records.length, 125)
    assert.equal(snapshot.selected_record_keys.length, 125)
    assert.equal(calls.length, 3)
    assert.equal(calls[1].searchParams.get('cursor'), 'second')
  } finally { globalThis.fetch = originalFetch }
})

test('snapshot loader rejects changed watermarks and incomplete pagination', async () => {
  const originalFetch = globalThis.fetch
  try {
    globalThis.fetch = async () => Response.json({ schema_version: 1, as_of: 'changed', records: [], total: 0, next_cursor: null })
    await assert.rejects(getPhoneSnapshot({}, 'pinned'), /snapshot changed/)
    globalThis.fetch = async () => Response.json({ schema_version: 1, as_of: 'pinned', records: [], total: 125, next_cursor: null })
    await assert.rejects(getPhoneSnapshot({}, 'pinned'), /snapshot is incomplete/)
  } finally { globalThis.fetch = originalFetch }
})
