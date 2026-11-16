import type { EntityProjection, MetricGroup, ObservationProjection, LegacyProjection } from '../../../shared/phone-benchmarks/types.ts'
import { metricSeriesKey, type PhoneActivity, type PhoneMetricSeries, type LegacyPhoneObservation } from './phoneBenchmarkTypes.ts'

export function projectedSeries(groups: MetricGroup[], projection: Pick<ObservationProjection, 'raw_metrics' | 'configurations'>) {
  const samples = new Map<string, Array<{ timestamp: number; value: number | null }>>()
  const series = groups.map(group => {
    const references = new Set(group.event_references.map(ref => JSON.stringify([ref.installation_id, ref.record_id, ref.event_index])))
    const events = projection.raw_metrics.filter(metric => metric.references.some(ref => references.has(JSON.stringify([ref.installation_id, ref.record_id, ref.event_index])))).map(metric => metric.event).sort((a, b) => a.timestamp_ms - b.timestamp_ms)
    const last = events.at(-1)
    const item: PhoneMetricSeries = {
      installation_id: group.installation_id, device: { installation_id: group.installation_id, hardware: group.hardware, os_version: group.os_version, app_version: group.app_version, build: group.build },
      configuration_id: group.configuration_id,
      configuration: group.configuration ?? projection.configurations.find(item => item.installation_id === group.installation_id && item.entity_id === group.configuration_id)?.data ?? null,
      stage: group.stage, outcome: group.outcome, dataset_revision: group.dataset_revision, protocol_revision: group.protocol_revision,
      name: group.name, unit: group.unit, scope: group.scope, source: group.source,
      observation_count: group.event_count, numeric_count: group.numeric.count, unavailable_count: group.unavailable_count,
      unavailable_reasons: group.unavailable_reasons.map(value => value.reason ?? 'Reason unreported'),
      min: group.numeric.min, mean: group.numeric.mean, max: group.numeric.max,
      latest: last?.value ?? null, latest_timestamp_ms: last?.timestamp_ms ?? 0,
      status_counts: Object.fromEntries(group.status_counts.map(value => [String(value.value), value.count])),
    }
    samples.set(metricSeriesKey(item), events.map(event => ({ timestamp: event.timestamp_ms, value: typeof event.value === 'number' ? event.value : null })))
    return item
  })
  return { series, samples }
}

const legacyMetadata = (record: LegacyPhoneObservation) => Object.fromEntries(record.batch.events.filter(event => typeof event.value === 'string').map(event => [event.name, event.value as string]))
/** Only explicit IDs within one installation connect historical runs. No timestamp guesses. */
export function legacyRunRecords(records: LegacyPhoneObservation[], runID: string, installationID: string | null, experimentID?: string) {
  const scoped = records.filter(item => (item.device?.installation_id ?? null) === installationID)
  const selected = new Set(scoped.filter(item => item.batch.run_id === runID && (installationID !== null || !experimentID || item.batch.experiment_id === experimentID)))
  if (installationID === null) return new Set([...selected].map(item => item.batch.experiment_id ?? item.batch.run_id)).size > 1 ? [] : [...selected]
  let changed = true
  while (changed) {
    changed = false
    const ids = new Set([...selected].map(item => item.batch.run_id))
    const native = new Set([...selected].map(item => legacyMetadata(item).native_run_id).filter(Boolean))
    const turns = new Set([...selected].map(item => legacyMetadata(item).turn_id).filter(Boolean))
    const parents = new Set([...selected].map(item => legacyMetadata(item).parent_run_id).filter(Boolean))
    for (const item of scoped) {
      if (selected.has(item)) continue
      const metadata = legacyMetadata(item)
      // Reused native counters with several explicit turn IDs cannot safely join.
      const linkedNative = metadata.native_run_id || item.batch.run_id
      const nativeTurns = new Set(scoped.filter(other => legacyMetadata(other).native_run_id === linkedNative).map(other => legacyMetadata(other).turn_id).filter(Boolean))
      const safeNative = nativeTurns.size <= 1
      if ((metadata.turn_id && turns.has(metadata.turn_id)) || (safeNative && (native.has(item.batch.run_id) || ids.has(metadata.native_run_id) || native.has(metadata.native_run_id))) || parents.has(item.batch.run_id) || (metadata.parent_run_id && ids.has(metadata.parent_run_id))) {
        selected.add(item); changed = true
      }
    }
  }
  return [...selected]
}
export function legacyConfiguration(records: LegacyPhoneObservation[]) {
  const configuration: Record<string, unknown> = { asr: null, reasoning: null }
  for (const item of records) {
    const metadata = legacyMetadata(item)
    const purpose = metadata.model_purpose === 'transcription' ? 'asr' : metadata.model_purpose === 'reasoning' ? 'reasoning' : null
    if (purpose) {
      const previous = configuration[purpose] as { name?: string; sha256?: string } | null
      if (!previous || previous.sha256 === metadata.model_sha256) configuration[purpose] = { name: metadata.model_name ?? null, sha256: metadata.model_sha256 ?? null }
      else configuration[purpose] = { name: 'Multiple reported models; inspect individual stages', sha256: null }
    }
  }
  return configuration
}
export function legacyActivities(projection: LegacyProjection): PhoneActivity[] {
  const groups = new Map<string, LegacyPhoneObservation[]>()
  for (const record of projection.records) {
    const key = JSON.stringify([record.device?.installation_id ?? null, record.batch.experiment_id, record.batch.run_id])
    groups.set(key, [...groups.get(key) ?? [], record])
  }
  return [...groups.values()].map(records => {
    const first = records[0]
    const linked = legacyRunRecords(projection.records, first.batch.run_id, first.device?.installation_id ?? null, first.batch.experiment_id)
    const metadata = legacyMetadata(first)
    return {
      entity_id: first.batch.run_id, installation_id: first.device?.installation_id ?? null, device: first.device,
      kind: 'legacy_run', status: null, timestamp_ms: Math.max(...records.map(item => item.batch.timestamp_ms)), started_at_ms: null, finished_at_ms: null,
      configuration_id: null, configuration: legacyConfiguration(linked), context: { ...metadata, ...(first.batch.experiment_id ? { experiment_id: first.batch.experiment_id } : {}) }, source: null,
      stage: metadata.model_purpose ?? null, metric_batch_count: records.length, event_count: records.reduce((total, item) => total + item.batch.events.length, 0), completeness: 'unknown' as const, metrics: [],
    }
  }).sort((a, b) => b.timestamp_ms - a.timestamp_ms)
}
export function projectedActivity(entity: EntityProjection, projection: ObservationProjection): PhoneActivity {
  const records = projection.records.filter(value => value.device.installation_id === entity.installation_id && value.record.kind === entity.kind && value.record.entity_id === entity.entity_id)
  const metrics = projectedSeries(entity.metric_groups, projection).series
  return {
    installation_id: entity.installation_id, device: entity.device, entity_id: entity.entity_id, kind: entity.kind,
    status: entity.status, timestamp_ms: Math.max(0, ...records.map(value => value.record.timestamp_ms)),
    started_at_ms: entity.started_at_ms, finished_at_ms: entity.finished_at_ms,
    configuration_id: entity.context.configuration_id ?? null,
    configuration: projection.configurations.find(value => value.installation_id === entity.installation_id && value.entity_id === entity.context.configuration_id)?.data ?? null,
    context: { ...entity.context }, source: entity.source, stage: entity.stage,
    metric_batch_count: new Set(entity.metric_groups.flatMap(group => group.event_references.map(ref => ref.record_id))).size,
    event_count: metrics.reduce((total, metric) => total + metric.observation_count, 0), completeness: entity.completeness, metrics,
  }
}
