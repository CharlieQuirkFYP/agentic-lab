import { projectObservations } from '../../../shared/phone-benchmarks/index.ts';
import type { PhoneDevice, PhoneObservation, PhoneMetricEvent, StoredPhoneObservation } from '../../../shared/phone-benchmarks/types.ts';

/** Entirely synthetic. These values must never be cited as device evidence. */
export const fixtureTime = Date.UTC(2026, 10, 13, 2, 0, 0);
export const configurationID = 'illustrative-whisper-qwen';
export const conversationID = 'illustrative-chat-current';
const device: PhoneDevice = { installation_id: 'illustrative-installation', hardware: 'Illustrative iPhone', os_version: 'Illustrative iOS', app_version: 'preview', build: 'fixture' };
export const fixtureRecords: StoredPhoneObservation[] = [];
let sequence = 0;
function add(kind: PhoneObservation['kind'], id: string, data: Record<string, unknown>, context: PhoneObservation['context'] = {}) {
  const record: PhoneObservation = { schema_version: 1, record_id: `illustrative-record-${sequence}`, producer_stream_id: 'illustrative-stream',
    record_sequence: sequence++, timestamp_ms: fixtureTime + sequence, kind, entity_id: id, revision: 0, context, data };
  fixtureRecords.push({ device, record, received_at_ms: fixtureTime + 60_000 });
  return record;
}
function finalize(kind: 'attempt' | 'conversation', id: string) {
  const field = kind === 'attempt' ? 'attempt_id' : 'conversation_id';
  const required = fixtureRecords.filter(({ record }) => record.entity_id === id || record.context[field] === id);
  const eventCount = required.reduce((sum, { record }) => sum + (record.kind === 'metric_batch' ? ((record.data.batch as { events: unknown[] }).events.length) : 0), 0);
  add('finalization', `final-${id}`, { status: 'complete', target_revision: 0, event_count: eventCount, dropped_events: 0,
    required_record_ids: required.map(({ record }) => record.record_id), record_count: required.length }, { [field]: id });
}
add('configuration', configurationID, {
  asr: { id: 'illustrative-whisper', name: 'Whisper Small', sha256: 'illustrative-asr-checksum' },
  reasoning: { id: 'illustrative-qwen', name: 'Qwen3.5 4B', sha256: 'illustrative-reasoning-checksum' },
  settings: { threads: 4 }, runtime: { revision: 'illustrative-runtime' },
});

function conversation(id: string, start: number, source: 'manual' | 'sweep', count: number, cancelled: boolean) {
  const context = { configuration_id: configurationID, conversation_id: id, ...(source === 'sweep' ? { sweep_id: 'illustrative-sweep' } : {}) };
  add('conversation', id, { status: cancelled ? 'cancelled' : 'succeeded', source, started_at_ms: start, finished_at_ms: start + 30_000, duration_ms: 30_000 }, context);
  for (let turn = 0; turn < count; turn++) {
    const turnID = `${id}-turn-${turn + 1}`, turnStart = start + turn * 10_000;
    const stopped = cancelled && turn === count - 1;
    add('turn', turnID, { status: stopped ? 'cancelled' : 'succeeded', turn_index: turn, input_kind: 'audio',
      started_at_ms: turnStart, finished_at_ms: turnStart + 9000, duration_ms: 9000 }, context);
    for (const stage of ['transcription', 'reasoning'] as const) {
      const attemptID = `${turnID}-${stage}`, offset = stage === 'reasoning' ? 1500 : 0;
      const elapsed = stage === 'transcription' ? [1100, 1350, 1200][turn] : [4800, 6400, 2500][turn];
      const status = stopped && stage === 'reasoning' ? 'cancelled' : 'succeeded';
      const attemptContext = { ...context, turn_id: turnID, attempt_id: attemptID };
      add('attempt', attemptID, { stage, status, attempt_index: 0, native_run_id: `native-${attemptID}`,
        started_at_ms: turnStart + offset, finished_at_ms: turnStart + offset + elapsed, duration_ms: elapsed }, { ...context, turn_id: turnID });
      const events: PhoneMetricEvent[] = [];
      const event = (name: string, value: PhoneMetricEvent['value'], unit: string, scope: string, source: string, ms: number, unavailable_reason?: string) => {
        events.push({ schema_version: 1, run_id: `native-${attemptID}`, sequence: events.length, timestamp_ms: ms, name, value, unit, scope, source,
          ...(unavailable_reason ? { unavailable_reason } : {}) });
      };
      for (let sample = 0; sample < 5; sample++) {
        const ms = turnStart + offset + sample * Math.min(500, elapsed / 4);
        event('resident_memory_bytes', (2048 + turn * 256 + (stage === 'reasoning' ? 512 : 0) + [0, 120, 240, 180, 100][sample]) * 1024 * 1024,
          'bytes', 'process', 'darwin.mach_task_basic_info', ms);
        event('cpu_percent', [120, 260, 330, 280, 160][sample] + turn * 12, 'percent_one_core', 'process', 'darwin.task_info', ms);
        event('thermal_state', turn > 0 && stage === 'reasoning' ? 'fair' : 'nominal', 'state', 'device', 'ios.ProcessInfo', ms);
      }
      event(`${stage}_duration_ms`, elapsed - 30, 'milliseconds', 'run', 'pheme.runtime', turnStart + offset);
      if (stage === 'reasoning') event('time_to_first_token_ms', 420 + turn * 100, 'milliseconds', 'run', 'pheme.runtime', turnStart + offset);
      event('gpu_utilization', null, 'percent', 'device', 'ios.unavailable', turnStart + offset, 'no_validated_gpu_provider');
      event('power', null, 'watts', 'device', 'ios.unavailable', turnStart + offset, 'no_validated_power_provider');
      add('metric_batch', `${attemptID}-batch`, { batch: { schema_version: 1, run_id: `native-${attemptID}`, sequence_number: 0,
        timestamp_ms: turnStart + offset, events } }, attemptContext);
      finalize('attempt', attemptID);
    }
  }
  finalize('conversation', id);
}
conversation(conversationID, fixtureTime, 'manual', 3, true);
conversation('illustrative-chat-saved', fixtureTime - 86_400_000, 'manual', 2, false);
add('sweep', 'illustrative-sweep', { status: 'succeeded', started_at_ms: fixtureTime - 172_800_000 });
conversation('illustrative-sweep-chat', fixtureTime - 172_800_000, 'sweep', 2, false);

export function snapshotFor(id: string | null, sweepID?: string, variant?: string) {
  let records = fixtureRecords.filter(({ record }) => record.kind === 'configuration' ||
    (id ? record.entity_id === id || record.context.conversation_id === id || (record.kind === 'finalization' && record.context.attempt_id?.startsWith(id))
      : record.entity_id === sweepID || record.context.sweep_id === sweepID || record.context.attempt_id?.startsWith('illustrative-sweep-chat')));
  if (['zero', 'single', 'missing', 'none'].includes(variant ?? '')) {
    records = structuredClone(records);
    let seen = 0;
    for (const { record } of records) {
      if (record.kind !== 'metric_batch') continue;
      const batch = record.data.batch as { events: PhoneMetricEvent[] };
      batch.events = batch.events.filter(event => !(variant === 'none' && event.name === 'resident_memory_bytes'));
      for (const event of batch.events) {
        if (event.name !== 'resident_memory_bytes') continue;
        if (variant === 'zero') event.value = 0;
        if (variant === 'missing' || variant === 'single' && seen++ > 0) {
          event.value = null; event.unavailable_reason = 'illustrative_sensor_gap';
        }
      }
    }
    for (const { record } of records) {
      if (record.kind !== 'finalization') continue;
      const required = new Set(record.data.required_record_ids as string[]);
      record.data.event_count = records.reduce((sum, item) => sum + (required.has(item.record.record_id) && item.record.kind === 'metric_batch'
        ? (item.record.data.batch as { events: PhoneMetricEvent[] }).events.length : 0), 0);
    }
  }
  const selected_record_keys = records.filter(({ record }) => record.kind !== 'configuration').map(({ device: currentDevice, record }) => ({ installation_id: currentDevice.installation_id, record_id: record.record_id }));
  return { records, selected_record_keys, as_of: `illustrative-${id ?? sweepID}-${variant ?? 'standard'}`, generation: 1, sequence };
}
export const allProjection = projectObservations(fixtureRecords);
