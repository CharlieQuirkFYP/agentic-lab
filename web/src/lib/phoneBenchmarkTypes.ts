import type { PhoneDevice, PhoneObservationPage, PhoneMetricBatch } from '../../../shared/phone-benchmarks/types.ts'

export type { PhoneDevice, PhoneObservationPage }
export interface PhoneFilters {
  installation_id?: string; configuration_id?: string; conversation_id?: string; sweep_id?: string;
  since_ms?: number; until_ms?: number; source?: string; stage?: string; outcome?: string;
}
export function parsePhoneFilters(params: URLSearchParams): PhoneFilters {
  const filters: PhoneFilters = {}
  for (const key of ['installation_id', 'configuration_id', 'conversation_id', 'sweep_id', 'source', 'stage', 'outcome'] as const) {
    const value = params.get(key)
    if (value) filters[key] = value
  }
  for (const key of ['since_ms', 'until_ms'] as const) {
    const value = params.get(key)
    if (value && Number.isFinite(Number(value))) filters[key] = Number(value)
  }
  return filters
}
export interface PhoneMetricSeries {
  installation_id: string | null; device: PhoneDevice | null;
  configuration_id: string | null; configuration: Record<string, unknown> | null;
  stage: string | null; dataset_revision: string | null; protocol_revision: string | null; outcome: string | null;
  name: string; unit: string; scope: string; source: string;
  observation_count: number; numeric_count: number; unavailable_count: number; unavailable_reasons: string[];
  min: number | null; mean: number | null; max: number | null;
  latest: number | string | boolean | null; latest_timestamp_ms: number; status_counts: Record<string, number>;
}
export interface PhoneActivity {
  installation_id: string | null; device: PhoneDevice | null; entity_id: string; kind: string; status: string | null;
  timestamp_ms: number; started_at_ms: number | null; finished_at_ms: number | null;
  configuration_id: string | null; configuration: Record<string, unknown> | null; context: Record<string, string>;
  source: string | null; stage: string | null; metric_batch_count: number; event_count: number;
  completeness: 'complete' | 'partial' | 'unknown'; metrics: PhoneMetricSeries[];
}
interface OutcomeCounts { total: number; outcomes: Record<string, number> }
export interface PhoneOverview {
  schema_version: 1; as_of: string; source: 'device_reported';
  coverage: {
    record_count: number; metric_batch_count: number; event_count: number; legacy_batch_count: number;
    legacy_conversation_count: null; first_timestamp_ms: number | null; last_timestamp_ms: number | null;
    last_received_at_ms: number | null; unresolved_record_count: number;
    finalization: { complete: number; partial: number; unknown: number; dropped_events: number }; retained_history: true;
  };
  counts: { configurations: number; conversations: OutcomeCounts; turns: OutcomeCounts; attempts: OutcomeCounts; sweeps: OutcomeCounts; jobs: OutcomeCounts };
  series: PhoneMetricSeries[]; current_conversation: PhoneActivity | null;
  recent_conversations: PhoneActivity[]; recent_runs: PhoneActivity[]; recent_sweeps: PhoneActivity[];
}
export interface LegacyPhoneObservation { device: PhoneDevice | null; batch: PhoneMetricBatch; received_at_ms: number }
export interface LegacyPhonePage { schema_version: 1; as_of: string; batches: LegacyPhoneObservation[]; total: number; next_cursor: string | null }

export function phoneQuery(filters: PhoneFilters, extra: Record<string, string | number | undefined> = {}): string {
  const query = new URLSearchParams()
  for (const [key, value] of Object.entries({ ...filters, ...extra })) if (value !== undefined && value !== '') query.set(key, String(value))
  return query.toString()
}

/** Serial refreshes coalesce bursts; disposal prevents stale responses after filter/navigation changes. */
export function createCoalescedRefresh<T>(load: (signal: AbortSignal) => Promise<T>, apply: (result: T) => void, fail: (error: unknown) => void) {
  let running = false
  let requested = false
  let disposed = false
  const controller = new AbortController()
  const refresh = async () => {
    if (disposed) return
    requested = true
    if (running) return
    running = true
    while (requested && !disposed) {
      requested = false
      try { const result = await load(controller.signal); if (!disposed) apply(result) }
      catch (error) { if (!disposed) fail(error) }
    }
    running = false
  }
  return { refresh, dispose() { disposed = true; requested = false; controller.abort() } }
}

export function metricSeriesKey(series: PhoneMetricSeries): string {
  return JSON.stringify([series.installation_id, series.device, series.configuration_id, series.configuration, series.stage, series.dataset_revision, series.protocol_revision, series.outcome, series.name, series.unit, series.scope, series.source])
}
export function numericSegments(points: Array<{ timestamp: number; value: number | null }>) {
  const segments: Array<Array<{ timestamp: number; value: number }>> = []
  let current: Array<{ timestamp: number; value: number }> = []
  for (const point of [...points].sort((a, b) => a.timestamp - b.timestamp)) {
    if (point.value === null || !Number.isFinite(point.value)) { if (current.length) segments.push(current); current = [] }
    else current.push({ timestamp: point.timestamp, value: point.value })
  }
  if (current.length) segments.push(current)
  return segments
}
