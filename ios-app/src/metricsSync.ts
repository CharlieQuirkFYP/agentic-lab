import type { Observation } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { MetricBatch, MetricsDevice, MetricsOutbox, MetricsUpload, QueueStats } from './metricsOutbox.ts';

export interface SyncState extends QueueStats {
  url: string; enabled: boolean; uploading: boolean; error: string;
  observation: Observation | null; latestJSON: string; droppedHostBatches: number;
}
interface Dependencies {
  poll(): Promise<Observation>;
  device(): Promise<MetricsDevice>;
  fetch: typeof fetch;
}

export function createMetricsSync(outbox: MetricsOutbox, dependencies: Dependencies) {
  const listeners = new Set<() => void>();
  let state: SyncState = { ...outbox.stats(), ...outbox.settings(), uploading: false, error: '',
    observation: null, latestJSON: '', droppedHostBatches: 0 };
  let collecting = false;
  let uploading = false;
  let pendingObservation: Observation | null = null;
  const retained: MetricBatch[] = [];

  function publish(patch: Partial<SyncState> = {}) {
    state = { ...state, ...outbox.stats(), ...outbox.settings(), ...patch };
    for (const listener of listeners) listener();
  }
  function persistRetained(device: MetricsDevice) {
    if (retained.length) {
      outbox.enqueue(retained.map(batch => ({ schema_version: 1, device, batch })));
      retained.length = 0;
    }
  }
  const message = (error: unknown) => error instanceof Error ? error.message : String(error);

  return {
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    getSnapshot: () => state,
    configure(url: string, enabled: boolean) {
      if (uploading) throw new Error('Wait for the current metrics upload to finish.');
      outbox.configure(url, enabled);
      publish({ error: '' });
    },
    async collect() {
      if (collecting) return;
      collecting = true;
      try {
        // Resolve metadata before draining: failure must not consume events.
        const device = await dependencies.device();
        persistRetained(device);
        if (!pendingObservation) pendingObservation = await dependencies.poll();
        const observation = pendingObservation;
        const batches = JSON.parse(observation.batchesJSON) as MetricBatch[];
        if (!Array.isArray(batches)) throw new Error('Native metrics did not return a batch array.');
        outbox.enqueue(batches.map(batch => ({ schema_version: 1, device, batch })));
        pendingObservation = null;
        publish({ observation, latestJSON: batches.length ? observation.batchesJSON.slice(0, 12000) : state.latestJSON });
      } catch (error) {
        const code = (error as { code?: string }).code;
        if (!['not_open', 'closed', 'native_unavailable'].includes(code ?? '')) publish({ error: message(error) });
      } finally { collecting = false; }
    },
    async record(batch: MetricBatch) {
      try {
        if (retained.length >= 128) {
          publish({ error: 'Metrics storage remains unavailable; host batch retention is full.', droppedHostBatches: state.droppedHostBatches + 1 });
          return;
        }
        retained.push(batch);
        const device = await dependencies.device();
        persistRetained(device);
        publish();
      } catch (error) { publish({ error: message(error) }); }
    },
    async flush() {
      if (uploading || !outbox.settings().enabled) return;
      uploading = true;
      publish({ uploading: true });
      try {
        const url = outbox.settings().url;
        for (const entry of outbox.pending()) {
          const controller = new AbortController();
          const timeout = setTimeout(() => controller.abort(), 10000);
          try {
            const response = await dependencies.fetch(`${url}/api/v1/phone/metrics`, {
              method: 'POST', headers: { 'Content-Type': 'application/json' },
              body: entry.payload, signal: controller.signal, redirect: 'error',
            });
            if (response.status !== 202) {
              const detail = await response.json().catch(() => ({})) as { error?: string };
              const error = detail.error ?? `Metrics upload returned HTTP ${response.status}.`;
              if (response.status >= 400 && response.status < 500 && ![408, 429].includes(response.status)) {
                outbox.block(entry.id, error);
                publish({ error });
                continue;
              }
              throw new Error(error);
            }
            const ack = await response.json() as { status?: string; experiment_id?: string; event_count?: number };
            const upload = JSON.parse(entry.payload) as MetricsUpload;
            if (ack.status !== 'accepted' || !ack.experiment_id || ack.event_count !== upload.batch.events.length) {
              throw new Error('Server did not acknowledge this metrics batch.');
            }
            outbox.acknowledge(entry.id);
            publish({ error: outbox.blockedError() ?? '' });
          } finally { clearTimeout(timeout); }
        }
      } catch (error) { publish({ error: `Waiting to sync: ${message(error)}` }); }
      finally { uploading = false; publish({ uploading: false }); }
    },
  };
}
