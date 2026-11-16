import type { AttemptStage, ExecutionStatus, PhoneObservationContext } from '../../shared/phone-benchmarks/types.ts';
import type { Inspection, LocalReply, LocalResult, Observation } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { BenchmarkStore } from './benchmarkStore.ts';
import type { MetricBatch, MetricsDevice, SyncSettings } from './metricsOutbox.ts';
import { replyMetrics, transcriptionMetadata } from './phoneMetricEvents.ts';

export type BenchmarkOwner = 'manual' | 'sweep' | 'acceptance';
interface Dependencies {
  poll(): Promise<Observation>; inspect(): Promise<Inspection>; device(): Promise<MetricsDevice>;
  cancel(): Promise<void>; settings(): SyncSettings; fetch: typeof fetch;
  legacyFlush?(): Promise<void>; now?: () => number; monotonicNow?: () => number;
}
interface Operation { owner: BenchmarkOwner; attemptID: string; stage: AttemptStage; signal?: AbortSignal }
const terminal = new Set(['succeeded', 'failed', 'cancelled', 'interrupted', 'skipped']);
const isReply = (value: unknown): value is LocalReply => !!value && typeof value === 'object' && 'reply' in value;
const isTranscript = (value: unknown): value is LocalResult => !!value && typeof value === 'object' && 'transcription' in value;

/** One collector is shared by root polling and all native operation boundaries. */
export function createBenchmarkCollector(store: BenchmarkStore, dependencies: Dependencies) {
  const now = dependencies.now ?? Date.now;
  const monotonic = dependencies.monotonicNow ?? (() => performance.now());
  let owner: BenchmarkOwner | null = null, nativeBusy = false;
  let collection: Promise<Observation | null> | null = null, uploading: Promise<void> | null = null;
  let pending: { observation: Observation; batches: { batch: MetricBatch; context: PhoneObservationContext }[] } | null = null;
  let lastObservation: Observation | null = null;
  let runtimeID: string | undefined;
  let manualConversation: string | null = null;
  let interrupted: string | null = null;
  let pendingSettlement: (() => void) | null = null;
  const resourceSequences = new Map<string, number>();
  for (const { record } of store.snapshot()) {
    if (record.kind !== 'metric_batch') continue;
    const batch = record.data.batch as MetricBatch;
    if (!batch?.run_id.startsWith('ios-resource:')) continue;
    const key = `${record.context.runtime_session_id}:${batch.run_id}`;
    resourceSequences.set(key, Math.max(resourceSequences.get(key) ?? 0, batch.sequence_number + 1));
  }
  let error = '';
  const dropped = new Map<string, { events: number; resources: number }>();
  // A JS context restart cannot recover Rust chat UI state. Mark interrupted
  // work explicitly, retaining planned sweep jobs for the runner's decision.
  let recoveryPending = true;
  function recoverStartup() { store.recoverInterrupted(); recoveryPending = false; }
  try { recoverStartup(); } catch { error = 'Interrupted observations need storage recovery before another operation can start.'; }
  function contextFor(attemptID?: string, runID?: string, runtime?: string): PhoneObservationContext {
    const attempt = attemptID ? store.latest('attempt', attemptID) : null;
    if (attempt) return { ...attempt.context, attempt_id: attempt.entity_id, ...(runtime ? { runtime_session_id: runtime } : {}) };
    if (runID) {
      const matches = store.snapshot().map(item => item.record).filter(record => record.kind === 'attempt' && record.data.native_run_id === runID && (!runtime || !record.context.runtime_session_id || record.context.runtime_session_id === runtime));
      const matched = matches.at(-1);
      if (matched) return { ...matched.context, attempt_id: matched.entity_id, ...(runtime ? { runtime_session_id: runtime } : {}) };
    }
    return runtime ? { runtime_session_id: runtime } : {};
  }
  function observeRuntime(next?: string) {
    if (!next) return;
    if (runtimeID && runtimeID !== next) { store.recoverInterrupted('runtime_replaced', next); manualConversation = null; }
    runtimeID = next;
  }
  async function collect(): Promise<Observation | null> {
    if (collection) return collection;
    collection = (async () => {
      try {
        store.setDevice(await dependencies.device());
        if (recoveryPending) recoverStartup();
        if (!pending) {
          const observation = await dependencies.poll();
          observeRuntime(observation.runtime_session_id);
          const raw = JSON.parse(observation.batchesJSON) as MetricBatch[];
          if (!Array.isArray(raw)) throw new Error('Native metrics did not return a batch array.');
          const batches = raw.map(batch => ({ batch, context: contextFor(batch.run_id, batch.run_id, observation.runtime_session_id) }));
          for (const sample of observation.resource_samples ?? []) {
            const timestamp = Math.floor(sample.timestamp_ms);
            const run = sample.native_run_id ?? `runtime:${sample.runtime_session_id}`;
            const key = `${sample.runtime_session_id}:ios-resource:${run}`;
            const sequence = resourceSequences.get(key) ?? 0;
            resourceSequences.set(key, sequence + 1);
            const batch: MetricBatch = { schema_version: 1, run_id: `ios-resource:${run}`, sequence_number: sequence, timestamp_ms: timestamp,
              events: sample.values.map((value, index) => ({ schema_version: 1, run_id: `ios-resource:${run}`, sequence: sequence * 1000 + index, timestamp_ms: timestamp, ...value })) };
            // Duration/interval and boundaries remain real measured metadata.
            batch.events.push({ schema_version: 1, run_id: batch.run_id, sequence: sequence * 1000 + batch.events.length, timestamp_ms: timestamp,
              name: 'sample_monotonic_ms', value: sample.monotonic_ms, unit: 'milliseconds', scope: 'process', source: 'native.ios.resource' });
            batch.events.push({ schema_version: 1, run_id: batch.run_id, sequence: sequence * 1000 + batch.events.length, timestamp_ms: timestamp,
              name: 'resource_stage', value: sample.stage, unit: 'status', scope: 'process', source: 'native.ios.resource' });
            batch.events.push({ schema_version: 1, run_id: batch.run_id, sequence: sequence * 1000 + batch.events.length, timestamp_ms: timestamp,
              name: 'sample_boundary', value: sample.boundary, unit: 'status', scope: 'process', source: 'native.ios.resource' });
            if (sample.interval_ms !== null) batch.events.push({ schema_version: 1, run_id: batch.run_id, sequence: sequence * 1000 + batch.events.length, timestamp_ms: timestamp,
              name: 'sample_interval_ms', value: sample.interval_ms, unit: 'milliseconds', scope: 'process', source: 'native.ios.resource' });
            batches.push({ batch, context: contextFor(sample.attempt_id, sample.native_run_id, sample.runtime_session_id) });
          }
          pending = { observation, batches };
        }
        store.recordBatches(pending.batches);
        lastObservation = pending.observation;
        const key = lastObservation.runtime_session_id ?? 'unknown';
        dropped.set(key, { events: lastObservation.observer.dropped_events, resources: lastObservation.resource_observer?.dropped_samples ?? 0 });
        pending = null;
        if (pendingSettlement) { pendingSettlement(); pendingSettlement = null; }
        error = '';
        return lastObservation;
      } catch (cause) {
        error = cause instanceof Error ? cause.message : 'Observation storage unavailable.';
        throw cause;
      } finally { collection = null; }
    })();
    return collection;
  }
  async function withOwnership<T>(next: BenchmarkOwner, work: () => Promise<T>): Promise<T> {
    if (owner) throw new Error(`The ${owner} operation owns the on-device runtime.`);
    if (next !== 'manual' && manualConversation && store.snapshot().some(item => item.record.kind === 'turn' && item.record.context.conversation_id === manualConversation && store.latest('turn', item.record.entity_id)?.data.status === 'running')) throw new Error('Finish or discard the reviewed manual turn before starting a benchmark.');
    owner = next;
    try { return await work(); } finally { owner = null; }
  }
  function attemptFinalization(attemptID: string, previousDrop: { events: number; resources: number }, missing: boolean) {
    const attempt = store.latest('attempt', attemptID)!;
    const required = store.snapshot().filter(item => item.record.entity_id === attemptID || item.record.context.attempt_id === attemptID).filter(item => item.record.kind !== 'finalization');
    const all = required.filter(item => item.record.kind === 'metric_batch');
    const after = dropped.get(attempt.context.runtime_session_id ?? runtimeID ?? 'unknown') ?? previousDrop;
    const lost = Math.max(0, after.events - previousDrop.events) + Math.max(0, after.resources - previousDrop.resources);
    store.finalize({ ...attempt.context, attempt_id: attemptID }, { status: missing || lost > 0 || !lastObservation ? 'partial' : 'complete',
      target_revision: attempt.revision, required_record_ids: required.map(item => item.record.record_id), record_count: required.length,
      event_count: all.reduce((count, item) => count + ((item.record.data.batch as MetricBatch).events?.length ?? 0), 0), dropped_events: lost,
      ...(missing ? { reason_code: 'collection_unavailable' } : lost ? { reason_code: 'native_buffer_overflow' } : {}) });
  }
  async function runOperation<T>(options: Operation, operation: (attemptID: string) => Promise<T>): Promise<T> {
    if (recoveryPending || pendingSettlement || pending) throw new Error('Recover pending observation storage before starting another operation.');
    if (owner && owner !== options.owner || nativeBusy) throw new Error('Another operation owns the on-device runtime.');
    const acquired = !owner;
    if (acquired) owner = options.owner;
    nativeBusy = true; interrupted = null;
    let result: T | undefined, failure: unknown, failed = false, missing = false;
    const before = dropped.get(runtimeID ?? 'unknown') ?? { events: 0, resources: 0 };
    const started = now();
    let nativeDuration = 0;
    try {
      const attempt = store.latest('attempt', options.attemptID);
      if (!attempt || terminal.has(String(attempt.data.status))) throw new Error('Register a new attempt before native inference.');
      store.assertCanStart();
      // Admission stays held through the final native drain and durable outcome.
      store.append('attempt', options.attemptID, runtimeID ? { runtime_session_id: runtimeID } : {}, { status: 'running', stage: options.stage, started_at_ms: started, native_run_id: options.attemptID });
      const nativeStarted = monotonic();
      try {
        if (options.signal?.aborted) throw Object.assign(new Error('Operation cancelled.'), { code: 'cancelled' });
        result = await operation(options.attemptID);
        if (isReply(result) && (result.execution_location !== 'device' || !result.reply.run_id || !result.reply.text.trim())) throw new Error('Invalid on-device reply.');
        if (isTranscript(result) && (result.execution_location !== 'device' || !result.transcription.run_id || !result.transcription.result.text.trim())) throw new Error('No accepted speech detected.');
      } catch (cause) { failure = cause; failed = true; }
      finally { nativeDuration = Math.max(0, monotonic() - nativeStarted); }
      try {
        // An in-flight root poll may have started before native completion.
        // Finish that drain, retry any retained payload, then poll once more
        // after settlement to include the final boundary and native counters.
        if (collection) await collection;
        if (pending) await collect();
        await collect();
      } catch { missing = true; }
      const context = contextFor(options.attemptID, options.attemptID, runtimeID);
      const hostBatches = !failed && isReply(result) ? [{ batch: replyMetrics(result, context.turn_id ?? options.attemptID), context }] :
        !failed && isTranscript(result) ? [{ batch: transcriptionMetadata(result, context.turn_id), context }] : [];
      const nativeError = failure as { code?: string; runID?: string } | undefined;
      const cancelled = interrupted === 'user_cancelled' || options.signal?.aborted || /cancel|abort/i.test(nativeError?.code ?? '');
      const status: ExecutionStatus = cancelled ? 'cancelled' : interrupted ? 'interrupted' : failed ? 'failed' : 'succeeded';
      const runID = isReply(result) ? result.reply.run_id : isTranscript(result) ? result.transcription.run_id : nativeError?.runID ?? options.attemptID;
      const duration = nativeDuration;
      const reason = interrupted ?? (cancelled ? (options.signal?.aborted ? 'user_cancelled' : 'native_cancelled') : 'native_failure');
      pendingSettlement = () => {
        store.recordBatches(hostBatches);
        if (!terminal.has(String(store.latest('attempt', options.attemptID)?.data.status))) store.completeAttempt(options.attemptID, status, { native_run_id: runID, duration_ms: duration, timer_boundary: 'native_call_including_bridge',
          ...(status !== 'succeeded' ? { reason_code: reason } : {}) });
        attemptFinalization(options.attemptID, before, missing);
      };
      pendingSettlement(); pendingSettlement = null;
      if (interrupted || cancelled) throw Object.assign(new Error('On-device operation interrupted or cancelled.'), { code: status });
      if (failed) throw failure;
      // If telemetry persistence fails, do not admit subsequent inference.
      if (missing) throw Object.assign(new Error('Native operation settled, but metrics storage needs recovery before continuing.'), { code: 'collection_unavailable' });
      return result as T;
    } finally { nativeBusy = false; if (acquired) owner = null; }
  }
  async function configuration() {
    store.setDevice(await dependencies.device());
    const state = await dependencies.inspect();
    observeRuntime(state.runtime_session_id);
    const effective = state.effective_configuration;
    const settings = effective?.asr?.settings ?? state.settings;
    const safeSettings = settings ? { modelID: settings.modelID, language: settings.language, threads: settings.threads, maxAudioSeconds: settings.maxAudioSeconds, metricsEnabled: settings.metricsEnabled,
      dictionary_count: settings.dictionary.length, dictionary_sha256: effective?.asr?.dictionary_sha256 ?? null } : null;
    return store.registerConfiguration({ asr: effective?.asr?.model ?? state.model, reasoning: effective?.reasoning?.model ?? state.reasoningModel,
      settings: { asr: safeSettings, reasoning: effective?.reasoning?.settings ?? state.reasoning?.conversation_config ?? null,
        reasoning_threads: effective?.reasoning?.threads ?? state.reasoning?.threads ?? null, reasoning_gpu_layers: effective?.reasoning?.gpu_layers ?? null },
      runtime: { asr: effective?.asr?.runtime ?? null, asr_capabilities: effective?.asr?.capabilities ?? state.snapshot.capabilities,
        reasoning: effective?.reasoning?.runtime ?? null, sampler: effective?.sampler ?? null, app_version: effective?.app_version ?? null, app_build: effective?.app_build ?? null },
      role_sha256: effective?.reasoning?.prompt_sha256 ?? null });
  }
  async function flush() {
    if (uploading) return uploading;
    uploading = (async () => {
      try {
        await dependencies.legacyFlush?.();
        const settings = dependencies.settings();
        if (!settings.enabled) return;
        const rows = store.pending(50);
        // Device metadata is part of the immutable archived envelope. Group by
        // that snapshot rather than rewriting old records after an app update.
        for (const deviceJSON of new Set(rows.map(row => row.device))) {
          const selected = rows.filter(row => row.device === deviceJSON);
          const body = JSON.stringify({ schema_version: 1, device: JSON.parse(deviceJSON), records: selected.map(row => JSON.parse(row.payload)) });
          if (body.length > 3 * 1024 * 1024) throw new Error('Observation upload chunk is too large.');
          const abort = new AbortController(), timeout = setTimeout(() => abort.abort(), 10000);
          try {
            const response = await dependencies.fetch(`${settings.url}/api/v1/phone/observations`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body, signal: abort.signal, redirect: 'error' });
            if (response.status !== 202) {
              const code = `observation_http_${response.status}`;
              if (response.status >= 400 && response.status < 500 && ![408, 429].includes(response.status)) { store.block(selected.map(row => row.record_id), code); continue; }
              throw new Error(code);
            }
            const ack = await response.json() as { status: string; record_count: number; accepted_record_ids: string[] };
            const ids = selected.map(row => row.record_id);
            if (ack.status !== 'accepted' || ack.record_count !== ids.length || !Array.isArray(ack.accepted_record_ids) || new Set(ack.accepted_record_ids).size !== ids.length || !ids.every(id => ack.accepted_record_ids.includes(id))) throw new Error('Server did not acknowledge this observation chunk.');
            store.acknowledge(ids);
          } finally { clearTimeout(timeout); }
        }
        error = '';
      } catch (cause) { error = cause instanceof Error ? cause.message : 'Waiting to sync observations.'; }
      finally { uploading = null; }
    })();
    return uploading;
  }
  function closeManualConversation(reason = 'new_chat') {
    if (!manualConversation) return;
    if (nativeBusy) throw new Error('Wait for native inference to settle before closing the conversation.');
    const conversation = store.latest('conversation', manualConversation);
    if (conversation && !terminal.has(String(conversation.data.status))) {
      const children = store.snapshot().map(item => item.record).filter(record => record.context.conversation_id === manualConversation);
      const turnIDs = new Set(children.filter(record => record.kind === 'turn').map(record => record.entity_id));
      for (const turnID of turnIDs) {
        const turn = store.latest('turn', turnID)!;
        if (!terminal.has(String(turn.data.status))) store.completeTurn(turnID, 'cancelled', { reason_code: 'conversation_reset' });
      }
      store.completeConversation(manualConversation, reason === 'new_chat' ? 'succeeded' : 'interrupted', { reason_code: reason });
      const finalized = [...new Set(children.filter(record => record.kind === 'attempt').map(record => record.entity_id))].map(id => store.latest('finalization', id));
      const required = store.snapshot().filter(item => item.record.entity_id === manualConversation || item.record.context.conversation_id === manualConversation).filter(item => item.record.kind !== 'finalization');
      store.finalize({ ...conversation.context, conversation_id: manualConversation }, { status: finalized.every(item => item?.data.status === 'complete') ? 'complete' : 'partial',
        target_revision: store.latest('conversation', manualConversation)!.revision, required_record_ids: required.map(item => item.record.record_id), record_count: required.length,
        event_count: finalized.reduce((n, item) => n + Number(item?.data.event_count ?? 0), 0), dropped_events: finalized.reduce((n, item) => n + Number(item?.data.dropped_events ?? 0), 0) });
    }
    manualConversation = null;
  }
  const controller = {
    store, collect, flush, configuration, runOperation, withOwnership, closeManualConversation,
    getOwner: () => owner, getError: () => error, getObservation: () => lastObservation,
    runtimeSessionID: () => runtimeID,
    async interrupt(reason = 'app_background') { if (nativeBusy) { interrupted = reason; await dependencies.cancel(); } },
    async beginManualTurn(inputKind: 'text' | 'audio') {
      if (owner && owner !== 'manual') throw new Error('Wait for the benchmark to finish before starting a chat turn.');
      const configurationID = await configuration();
      if (!manualConversation) manualConversation = store.registerConversation({ configuration_id: configurationID, ...(runtimeID ? { runtime_session_id: runtimeID } : {}) }, { source: 'manual' });
      const index = new Set(store.snapshot().filter(item => item.record.kind === 'turn' && item.record.context.conversation_id === manualConversation).map(item => item.record.entity_id)).size;
      const context = { configuration_id: configurationID, conversation_id: manualConversation, ...(runtimeID ? { runtime_session_id: runtimeID } : {}) };
      return store.registerTurn(context, { status: 'running', source: 'manual', input_kind: inputKind, turn_index: index, started_at_ms: now() });
    },
    async manualAttempt<T>(turnID: string, stage: AttemptStage, work: (attemptID: string) => Promise<T>, signal?: AbortSignal) {
      const turn = store.latest('turn', turnID);
      if (!turn || terminal.has(String(turn.data.status))) throw new Error('This manual turn is no longer active. Start a new chat.');
      const attemptID = store.registerAttempt({ ...turn.context, turn_id: turnID }, { source: 'manual', stage });
      return runOperation({ owner: 'manual', attemptID, stage, signal }, work);
    },
    finishManualTurn(turnID: string, status: ExecutionStatus) { const turn = store.latest('turn', turnID); if (turn && !terminal.has(String(turn.data.status))) store.completeTurn(turnID, status); },
    recordHost(batch: MetricBatch, turnID?: string) {
      // Correlated operations already persist their host metadata before their
      // terminal record. Legacy screen callbacks must not double-count them.
      if (turnID && store.latest('turn', turnID)) return;
      store.recordBatches([{ batch, context: runtimeID ? { runtime_session_id: runtimeID } : {} }]);
    },
  };
  return controller;
}
export type BenchmarkController = ReturnType<typeof createBenchmarkCollector>;
