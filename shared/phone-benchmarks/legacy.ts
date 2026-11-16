import type { LegacyPhoneObservation, LegacyProjection, ObservationFilters, PhoneDevice, ProjectedMetric } from './types.ts';
import { canonical, groupMetrics, ObservationConflictError } from './projection.ts';

/** Legacy native IDs are only meaningful within their original installation/container. */
export function projectLegacyObservations(input: readonly LegacyPhoneObservation[], filters: ObservationFilters = {}): LegacyProjection {
  const retained = new Map<string, LegacyPhoneObservation>();
  let duplicates = 0;
  for (const item of input) {
    const installation = item.device?.installation_id ?? `unknown:${item.batch.experiment_id ?? item.batch.run_id}`;
    const identity = canonical([installation, item.batch.experiment_id ?? null, item.batch.run_id, item.batch.sequence_number]);
    const previous = retained.get(identity);
    if (previous) {
      if (canonical(previous.batch) !== canonical(item.batch)) throw new ObservationConflictError(installation, `legacy batch ${identity}`);
      duplicates++;
    } else retained.set(identity, item);
  }
  const metadata = new Map<string, Record<string, string>>();
  const container = (item: LegacyPhoneObservation) => canonical([item.device?.installation_id ?? null, item.batch.experiment_id ?? item.batch.run_id]);
  for (const item of retained.values()) {
    const values = metadata.get(canonical([container(item), item.batch.run_id])) ?? {};
    for (const event of item.batch.events) if (typeof event.value === 'string') values[event.name] = event.value;
    metadata.set(canonical([container(item), item.batch.run_id]), values);
  }
  const raw: ProjectedMetric[] = [];
  const records: LegacyPhoneObservation[] = [];
  for (const item of retained.values()) {
    if (filters.installation_id && item.device?.installation_id !== filters.installation_id ||
      filters.configuration_id || filters.conversation_id || filters.sweep_id || filters.source || filters.outcome ||
      filters.since_ms !== undefined && item.batch.timestamp_ms < filters.since_ms ||
      filters.until_ms !== undefined && item.batch.timestamp_ms > filters.until_ms) continue;
    const installation = item.device?.installation_id ?? `unknown:${item.batch.experiment_id ?? item.batch.run_id}`;
    const own = metadata.get(canonical([container(item), item.batch.run_id])) ?? {};
    // Explicit native_run_id links may bridge legacy experiment containers only
    // when a known installation and an explicit turn ID make attribution possible.
    const parents = item.device ? [...retained.values()].filter(parent => parent.device?.installation_id === item.device?.installation_id)
      .map(parent => metadata.get(canonical([container(parent), parent.batch.run_id])) ?? {})
      .filter(values => values.native_run_id === item.batch.run_id) : [];
    const parentFields = ['turn_id', 'model_name', 'model_sha256', 'model_purpose', 'native_run_id'] as const;
    const unambiguous = parents.length > 0 && parents.every(values => parentFields.every(field => values[field] === parents[0][field]));
    const parent = unambiguous ? parents[0] : {};
    const values = { ...parent, ...own };
    const stage = values.model_purpose === 'transcription' || values.model_purpose === 'reasoning' ? values.model_purpose : null;
    if (filters.stage && stage !== filters.stage) continue;
    const device: PhoneDevice = item.device ?? { installation_id: installation, hardware: 'unknown', os_version: 'unknown', app_version: 'unknown', build: 'unknown' };
    const configuration: Record<string, unknown> | null = values.model_name || values.model_sha256 ? {
      asr: stage === 'transcription' ? { name: values.model_name ?? '', sha256: values.model_sha256 ?? '' } : null,
      reasoning: stage === 'reasoning' ? { name: values.model_name ?? '', sha256: values.model_sha256 ?? '' } : null,
    } : null;
    records.push(item);
    item.batch.events.forEach((event, eventIndex) => raw.push({
      event, device, installation_id: installation,
      configuration, parent_run_id: values.parent_run_id ?? null, native_run_id: values.native_run_id ?? null,
      context: values.turn_id ? { turn_id: values.turn_id } : {}, stage,
      outcome: null, completeness: 'unknown', dataset_revision: null, protocol_revision: null,
      references: [{ installation_id: installation,
        record_id: `legacy:${canonical([item.batch.experiment_id ?? null, item.batch.run_id, item.batch.sequence_number])}`,
        event_index: eventIndex, run_id: event.run_id, sequence: event.sequence }],
    }));
  }
  return { records, raw_metrics: raw, metric_groups: groupMetrics(raw), batch_count: records.length,
    event_count: raw.length, duplicate_batches: duplicates, canonical_duplicate_batches: 0, conversation_count: null,
    note: 'Legacy batches establish no conversation boundaries or telemetry finalization. Unknown installation containers remain isolated. Explicit turn/native-run links are attribution only; no execution outcome is inferred.' };
}
