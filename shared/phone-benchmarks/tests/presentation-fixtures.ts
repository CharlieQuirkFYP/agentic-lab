import type { PhoneDevice, PhoneMetricEvent, PhoneObservation, StoredPhoneObservation } from '../types.ts';

/** Small synthetic archive builder. No inference, device or measurement claims. */
export function presentationFixture() {
  let sequence = 0;
  const device: PhoneDevice = { installation_id: 'fixture-phone', hardware: 'fixture-hardware', os_version: 'fixture-os', app_version: 'fixture-app', build: 'fixture-build' };
  const records: StoredPhoneObservation[] = [];
  function add(kind: PhoneObservation['kind'], id: string, data: Record<string, unknown>, context: PhoneObservation['context'] = {}) {
    const previous = records.findLast(item => item.record.kind === kind && item.record.entity_id === id)?.record;
    const record: PhoneObservation = { schema_version: 1, record_id: `record-${sequence}`, producer_stream_id: 'fixture-stream',
      record_sequence: sequence++, timestamp_ms: 1000 + sequence, kind, entity_id: id, revision: previous ? previous.revision + 1 : 0,
      data: { ...previous?.data, ...data }, context: { ...previous?.context, ...context } };
    records.push({ device: { ...device }, record, received_at_ms: 2000 + sequence }); return record;
  }
  function configuration(id = 'config', data: Record<string, unknown> = {}) {
    return add('configuration', id, { asr: { id: 'asr', name: 'Fixture ASR', sha256: 'asr-hash' },
      reasoning: { id: 'reply', name: 'Fixture Reply', sha256: 'reply-hash' }, settings: { threads: 2 }, runtime: { revision: 'fixture-runtime' }, ...data });
  }
  function conversation(id = 'conversation', data: Record<string, unknown> = {}, context: PhoneObservation['context'] = {}) {
    return add('conversation', id, { status: 'succeeded', source: 'manual', started_at_ms: 1000, finished_at_ms: 20000,
      dataset_revision: 'dataset-v1', protocol_revision: 'protocol-v1', ...data }, { configuration_id: 'config', ...context });
  }
  function turn(id = 'turn', input_kind = 'audio', context: PhoneObservation['context'] = {}, data: Record<string, unknown> = {}) {
    return add('turn', id, { status: 'succeeded', input_kind, turn_index: sequence, started_at_ms: 1000, finished_at_ms: 10000, ...data },
      { conversation_id: 'conversation', ...context });
  }
  function metric(name: string, value: PhoneMetricEvent['value'], timestamp_ms = 1000, extra: Partial<PhoneMetricEvent> = {}): PhoneMetricEvent {
    return { schema_version: 1, name, value, timestamp_ms, sequence: 0, run_id: 'run', unit: 'bytes', scope: 'process',
      source: 'fixture-native', ...extra };
  }
  function finalize(id: string) {
    const attempt = records.findLast(item => item.record.kind === 'attempt' && item.record.entity_id === id)!.record;
    const required = records.filter(item => item.record.kind !== 'finalization' && (item.record.kind === 'attempt' && item.record.entity_id === id || item.record.context.attempt_id === id));
    const event_count = required.reduce((sum, item) => sum + ((item.record.data.batch as { events?: unknown[] } | undefined)?.events?.length ?? 0), 0);
    add('finalization', `final-${id}`, { status: 'complete', target_revision: attempt.revision,
      required_record_ids: required.map(item => item.record.record_id), record_count: required.length, dropped_events: 0, event_count }, { attempt_id: id });
  }
  function attempt(id: string, stage: string, duration_ms: number | null, options: {
    turn?: string; context?: PhoneObservation['context']; status?: string; complete?: boolean; events?: PhoneMetricEvent[];
    data?: Record<string, unknown>;
  } = {}) {
    const record = add('attempt', id, { stage, duration_ms, status: options.status ?? 'succeeded', started_at_ms: 1000,
      finished_at_ms: duration_ms === null ? null : 1000 + duration_ms, timer_boundary: 'native_call_including_bridge', ...options.data },
    { turn_id: options.turn ?? 'turn', ...options.context });
    if (options.events?.length) add('metric_batch', `batch-${id}`, { batch: { schema_version: 1, run_id: `run-${id}`, sequence_number: 0, timestamp_ms: 1000,
      events: options.events.map((event, sequence) => ({ ...event, run_id: `run-${id}`, sequence })) } }, { attempt_id: id });
    if (options.complete !== false) finalize(id);
    return record;
  }
  configuration(); conversation();
  return { records, device, add, configuration, conversation, turn, attempt, metric, finalize };
}
