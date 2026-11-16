import { AppState } from 'react-native';
import { openDatabaseSync } from 'expo-sqlite';
import Pheme from '../modules/pheme-native/src/PhemeNativeModule';
import type { LocalReply, LocalResult } from '../modules/pheme-native/src/PhemeNative.types';
import { createMetricsOutbox, type MetricsDevice } from './metricsOutbox';
import { createMetricsSync } from './metricsSync';
import { replyMetrics, transcriptionMetadata } from './phoneMetricEvents';
import { createBenchmarkStore } from './benchmarkStore';
import { createBenchmarkCollector } from './benchmarkCollector';

const db = openDatabaseSync('voice-recordings.db');
const outbox = createMetricsOutbox(db, process.env.EXPO_PUBLIC_METRICS_URL ?? '');
const store = createBenchmarkStore(db, { installationID: outbox.settings().installationID });
let device: Promise<MetricsDevice> | null = null;
const getDevice = () => {
  device ??= Pheme.deviceInfo().then(info => ({ ...info, installation_id: outbox.settings().installationID }))
    .catch(error => { device = null; throw error; });
  return device;
};
export const Benchmark = createBenchmarkCollector(store, {
  poll: Pheme.poll, inspect: Pheme.inspect, cancel: Pheme.cancel, device: getDevice,
  settings: outbox.settings, fetch: (...args) => fetch(...args), legacyFlush: () => Metrics.flush(),
});

// Preserve old pending envelopes and their exact retry identities. New native
// observations are archived by the shared collector and uploaded to /observations.
export const Metrics = createMetricsSync(outbox, {
  poll: async () => {
    const observation = await Benchmark.collect();
    if (!observation) throw new Error('Native observation unavailable.');
    return { ...observation, batchesJSON: '[]' };
  },
  device: getDevice, fetch: (...args) => fetch(...args),
});
let users = 0;
let stop: (() => void) | null = null;
// One root subscription, also safe under development StrictMode remounts.
export function startMetricsSync() {
  users++;
  if (!stop) {
    const collect = () => { if (AppState.currentState === 'active') void Metrics.collect(); };
    const flush = () => { if (AppState.currentState === 'active' && !Benchmark.getOwner()) void Benchmark.flush(); };
    collect(); flush();
    const polling = setInterval(collect, 1000), uploading = setInterval(flush, 5000);
    const lifecycle = AppState.addEventListener('change', state => {
      if (state === 'active') { collect(); flush(); }
      else void Benchmark.interrupt().catch(() => {});
    });
    stop = () => { clearInterval(polling); clearInterval(uploading); lifecycle.remove(); };
  }
  return () => { users--; if (!users) { stop?.(); stop = null; } };
}
export function recordTranscriptionMetadata(result: LocalResult, turnID?: string) {
  Benchmark.recordHost(transcriptionMetadata(result, turnID), turnID);
}
export function recordReplyMetrics(result: LocalReply, turnID: string) {
  Benchmark.recordHost(replyMetrics(result, turnID), turnID);
}
