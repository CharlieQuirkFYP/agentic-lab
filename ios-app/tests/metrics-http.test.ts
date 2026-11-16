import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { createMetricsOutbox, type MetricsConnection, type MetricsUpload } from '../src/metricsOutbox.ts';
import { createMetricsSync } from '../src/metricsSync.ts';

const url = process.env.PHEME_METRICS_TEST_URL;
test('optional live Go boundary stores app outbox batches and notifies the dashboard', { skip: !url }, async () => {
  assert.ok(url);
  const server = new URL(url);
  assert.equal(server.hostname, '127.0.0.1');
  assert.notEqual(server.port, '8080', 'Use an isolated test API/database, never the live phone collector.');
  const before = await (await fetch(`${url}/api/v1/phone/metrics`)).json() as { total: number };
  assert.equal(before.total, 0, 'The integration test requires an empty disposable database.');
  const db = new DatabaseSync(':memory:');
  const adapter: MetricsConnection = {
    execSync: sql => db.exec(sql),
    runSync: (sql, ...params) => db.prepare(sql).run(...params),
    getFirstSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).get(...params) as T ?? null,
    getAllSync: <T>(sql: string, ...params: (string | number | null)[]) => db.prepare(sql).all(...params) as T[],
  };
  const socket = new WebSocket(`${url.replace('http:', 'ws:')}/api/v1/dashboard/events`);
  try {
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true });
      socket.addEventListener('error', () => reject(new Error('Dashboard socket failed')), { once: true });
    });
    const notified = new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('No dashboard update arrived')), 5000);
      socket.addEventListener('message', event => {
        if (JSON.parse(String(event.data)).type === 'metrics.updated') { clearTimeout(timeout); resolve(); }
      }, { once: true });
    });
    const outbox = createMetricsOutbox(adapter, url);
    const upload: MetricsUpload = {
      schema_version: 1, device: { installation_id: outbox.settings().installationID,
        hardware: 'integration-fixture-not-a-phone', os_version: 'fixture', app_version: 'fixture', build: 'fixture' },
      batch: { schema_version: 1, run_id: 'integration-fixture', sequence_number: 0, timestamp_ms: 1000,
        events: [
          { schema_version: 1, run_id: 'integration-fixture', sequence: 0, timestamp_ms: 1000, name: 'reasoning_duration_ms', value: 123, unit: 'milliseconds', scope: 'run', source: 'integration_fixture' },
          { schema_version: 1, run_id: 'integration-fixture', sequence: 1, timestamp_ms: 1000, name: 'whole_device_power_watts', value: null, unit: 'watts', scope: 'device', source: 'integration_fixture', unavailable_reason: 'Fixture has no power meter' },
        ] },
    };
    outbox.enqueue([upload]);
    const original = outbox.pending()[0].payload;
    const sync = createMetricsSync(outbox, { device: async () => upload.device, poll: async () => { throw new Error('Unused'); }, fetch });
    await sync.flush();
    assert.equal(outbox.stats().pending, 0, sync.getSnapshot().error);
    await notified;
    const retried = await fetch(`${url}/api/v1/phone/metrics`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: original });
    assert.equal(retried.status, 202);
    const { experiment_id: id } = await retried.json() as { experiment_id: string };
    const page = await (await fetch(`${url}/api/v1/phone/metrics`)).json() as { total: number; batches: unknown[] };
    assert.equal(page.total, 1);
    const summary = await (await fetch(`${url}/api/v1/experiments/${id}/summary`)).json() as {
      result_source: string; result: unknown; metrics: { name: string; mean: number | null; unavailable_count: number }[];
    };
    assert.equal(summary.result_source, 'device_reported');
    assert.equal(summary.result, null);
    assert.equal(summary.metrics.find(m => m.name === 'reasoning_duration_ms')?.mean, 123);
    assert.equal(summary.metrics.find(m => m.name === 'whole_device_power_watts')?.mean, null);
    assert.equal(summary.metrics.find(m => m.name === 'whole_device_power_watts')?.unavailable_count, 1);
    const exported = await (await fetch(`${url}/api/v1/experiments/${id}/export?format=json`)).json() as { batches: unknown[] };
    assert.equal(exported.batches.length, 1);
  } finally { socket.close(); db.close(); }
});
