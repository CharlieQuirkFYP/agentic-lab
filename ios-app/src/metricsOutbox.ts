export type MetricValue = number | string | boolean | null;
export interface MetricEvent {
  schema_version: 1; run_id: string; sequence: number; timestamp_ms: number;
  name: string; value: MetricValue; unit: string; scope: string; source: string;
  unavailable_reason?: string;
}
export interface MetricBatch {
  schema_version: 1; run_id: string; sequence_number: number; timestamp_ms: number;
  events: MetricEvent[];
}
export interface MetricsDevice {
  installation_id: string; hardware: string; os_version: string; app_version: string; build: string;
}
export interface MetricsUpload { schema_version: 1; device: MetricsDevice; batch: MetricBatch }
type Parameter = string | number | null;
export interface MetricsConnection {
  execSync(sql: string): void;
  runSync(sql: string, ...params: Parameter[]): unknown;
  getFirstSync<T>(sql: string, ...params: Parameter[]): T | null;
  getAllSync<T>(sql: string, ...params: Parameter[]): T[];
}
export interface OutboxEntry { id: string; payload: string }
export interface SyncSettings { url: string; enabled: boolean; installationID: string }
export interface QueueStats { pending: number; blocked: number; acknowledged: number; lastUpload: number | null }

const MAX_BATCHES = 2000;
const MAX_BYTES = 8 * 1024 * 1024;

export function normalizeMetricsURL(value: string): string {
  const url = new URL(value.trim());
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
      url.search || url.hash || !['', '/', '/api/v1', '/api/v1/'].includes(url.pathname)) {
    throw new Error('Use your server address, for example http://192.168.1.40:8080.');
  }
  const host = url.hostname.toLowerCase();
  const octets = host.split('.').map(Number);
  const ipv4 = octets.length === 4 && octets.every(n => Number.isInteger(n) && n >= 0 && n <= 255);
  const local = host === 'localhost' || host === '[::1]' || host.endsWith('.local') ||
    ipv4 && (octets[0] === 127 || octets[0] === 10 || octets[0] === 192 && octets[1] === 168 ||
      octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31 || octets[0] === 169 && octets[1] === 254);
  if (url.protocol === 'http:' && !local) throw new Error('HTTP is allowed only on a trusted local network. Use HTTPS elsewhere.');
  return url.origin;
}

export function createMetricsOutbox(db: MetricsConnection, defaultURL = '') {
  db.execSync(`CREATE TABLE IF NOT EXISTS metric_outbox (
    id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL, created_at INTEGER NOT NULL,
    blocked INTEGER NOT NULL DEFAULT 0, error TEXT
  );
  CREATE TABLE IF NOT EXISTS metric_sync_settings (
    id INTEGER PRIMARY KEY CHECK(id=1), url TEXT NOT NULL, enabled INTEGER NOT NULL,
    installation_id TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0, last_upload INTEGER
  )`);
  const url = defaultURL ? normalizeMetricsURL(defaultURL) : '';
  db.runSync('INSERT OR IGNORE INTO metric_sync_settings(id,url,enabled,installation_id) VALUES(1,?,?,?)',
    url, url ? 1 : 0, `install-${Date.now()}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`);

  return {
    settings(): SyncSettings {
      const row = db.getFirstSync<{ url: string; enabled: number; installation_id: string }>('SELECT * FROM metric_sync_settings WHERE id=1');
      if (!row) throw new Error('Metrics settings are missing.');
      return { url: row.url, enabled: row.enabled === 1, installationID: row.installation_id };
    },
    configure(value: string, enabled: boolean) {
      const url = value.trim() ? normalizeMetricsURL(value) : '';
      if (enabled && !url) throw new Error('Set the metrics server address first.');
      db.runSync('UPDATE metric_sync_settings SET url=?,enabled=? WHERE id=1', url, enabled ? 1 : 0);
    },
    enqueue(uploads: MetricsUpload[]) {
      // A drain is committed as a group. Storage failure leaves its exact
      // payload with the collector for retry, rather than discarding events.
      db.execSync('BEGIN IMMEDIATE');
      try {
        for (const upload of uploads) {
          const id = JSON.stringify([upload.batch.run_id, upload.batch.sequence_number]);
          const payload = JSON.stringify(upload);
          const existing = db.getFirstSync<{ payload: string }>('SELECT payload FROM metric_outbox WHERE id=?', id);
          if (existing && existing.payload !== payload) throw new Error('Metrics batch identity has different content.');
          db.runSync('INSERT OR IGNORE INTO metric_outbox(id,payload,created_at) VALUES(?,?,?)', id, payload, Date.now());
        }
        const size = db.getFirstSync<{ count: number; bytes: number }>('SELECT COUNT(*) AS count,COALESCE(SUM(length(CAST(payload AS BLOB))),0) AS bytes FROM metric_outbox');
        if (size && (size.count > MAX_BATCHES || size.bytes > MAX_BYTES)) {
          throw new Error('Metrics queue is full. Sync pending batches before collecting more.');
        }
        db.execSync('COMMIT');
      } catch (error) { db.execSync('ROLLBACK'); throw error; }
    },
    pending(): OutboxEntry[] {
      return db.getAllSync<OutboxEntry>('SELECT id,payload FROM metric_outbox WHERE blocked=0 ORDER BY created_at,id LIMIT 25');
    },
    acknowledge(id: string) {
      db.execSync('BEGIN IMMEDIATE');
      try {
        if (db.getFirstSync('SELECT id FROM metric_outbox WHERE id=?', id)) {
          db.runSync('DELETE FROM metric_outbox WHERE id=?', id);
          db.runSync('UPDATE metric_sync_settings SET acknowledged=acknowledged+1,last_upload=? WHERE id=1', Date.now());
        }
        db.execSync('COMMIT');
      } catch (error) { db.execSync('ROLLBACK'); throw error; }
    },
    block(id: string, error: string) { db.runSync('UPDATE metric_outbox SET blocked=1,error=? WHERE id=?', error, id); },
    stats(): QueueStats {
      const row = db.getFirstSync<{ pending: number; blocked: number }>(
        'SELECT COALESCE(SUM(1-blocked),0) AS pending,COALESCE(SUM(blocked),0) AS blocked FROM metric_outbox');
      const settings = db.getFirstSync<{ acknowledged: number; last_upload: number | null }>('SELECT acknowledged,last_upload FROM metric_sync_settings WHERE id=1');
      return { pending: row?.pending ?? 0, blocked: row?.blocked ?? 0, acknowledged: settings?.acknowledged ?? 0, lastUpload: settings?.last_upload ?? null };
    },
    blockedError(): string | null {
      return db.getFirstSync<{ error: string }>('SELECT error FROM metric_outbox WHERE blocked=1 ORDER BY created_at LIMIT 1')?.error ?? null;
    },
  };
}
export type MetricsOutbox = ReturnType<typeof createMetricsOutbox>;
