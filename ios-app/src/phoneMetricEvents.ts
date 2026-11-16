import type { LocalReply, LocalResult } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { MetricBatch, MetricValue } from './metricsOutbox.ts';

function hostBatch(runID: string, values: Record<string, MetricValue>): MetricBatch {
  const timestamp = Date.now();
  const units: Record<string, string> = { asr_threads: 'count', max_audio_seconds: 'seconds' };
  return {
    schema_version: 1, run_id: runID, sequence_number: 0, timestamp_ms: timestamp,
    events: Object.entries(values).map(([name, value], sequence) => ({
      schema_version: 1, run_id: runID, sequence, timestamp_ms: timestamp,
      name, value, unit: units[name] ?? (name.endsWith('_ms') ? 'milliseconds' : 'status'),
      scope: 'run', source: 'native.ios.host',
      ...(value === null ? { unavailable_reason: 'Not reported by the native runtime.' } : {}),
    })),
  };
}

export function transcriptionMetadata(result: LocalResult, turnID?: string): MetricBatch {
  return hostBatch(`ios-asr-metadata:${result.transcription.run_id}`, {
    native_run_id: result.transcription.run_id, model_name: result.model.name,
    model_sha256: result.model.sha256, model_id: result.model.id, model_purpose: 'transcription',
    transcription_duration_ms: result.transcription.result.processing_time_ms ?? null, transcription_status: result.transcription.result.status,
    execution_location: result.execution_location, asr_threads: result.settings.threads,
    max_audio_seconds: result.settings.maxAudioSeconds, ...(turnID ? { turn_id: turnID } : {}),
  });
}

export function replyMetrics(result: LocalReply, turnID: string): MetricBatch {
  // Native reply counters restart with the engine; the chat turn distinguishes
  // observations across process/model reloads while retaining the native ID.
  const batch = hostBatch(`ios-reply:${turnID}:${result.reply.run_id}`, {
    native_run_id: result.reply.run_id, turn_id: turnID, model_name: result.model.name,
    model_sha256: result.model.sha256, model_id: result.model.id, model_purpose: 'reasoning',
    execution_location: result.execution_location,
    reasoning_duration_ms: result.reply.processing_time_ms ?? null,
    reply_status: 'completed',
  });
  for (const [name, unit] of [['whole_device_power_watts', 'watts'], ['energy_joules', 'joules']]) {
    batch.events.push({ schema_version: 1, run_id: batch.run_id, sequence: batch.events.length,
      timestamp_ms: batch.timestamp_ms, name, value: null, unit, scope: 'device', source: 'native.ios.host',
      unavailable_reason: 'No verified iOS whole-device power or energy provider is configured.' });
  }
  return batch;
}
