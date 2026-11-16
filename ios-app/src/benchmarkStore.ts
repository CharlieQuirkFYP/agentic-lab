import type { ExecutionStatus, ObservationKind, ObservationSource, PhoneDevice, PhoneObservation, PhoneObservationContext, RecordReference, StoredPhoneObservation } from '../../shared/phone-benchmarks/types.ts';
import type { MetricBatch, MetricsConnection } from './metricsOutbox.ts';

type Data = Record<string, unknown>;
type Row = { record_id: string; payload: string; device: string; created_at: number };
interface Options { installationID: string; uuid?: () => string; now?: () => number; maxRecords?: number; maxBytes?: number; reserveRecords?: number; reserveBytes?: number }
export interface ResultsScope { conversation_id?: string; sweep_id?: string; configuration_id?: string }
export interface ResultsHistoryOptions {
  source?: ObservationSource; sweep_id?: string; query?: string; since_ms?: number; until_ms?: number; limit?: number; cursor?: string;
}
export interface ResultsHistoryItem {
  entity_id: string; source: ObservationSource | null; status: string | null; started_at_ms: number;
  configuration_id: string | null; asr_name: string | null; reasoning_name: string | null;
}
export interface ResultsSnapshot {
  records: StoredPhoneObservation[]; selected_record_keys: RecordReference[];
  as_of: string; generation: number; sequence: number;
}
const contextKinds = { conversation_id: 'conversation', sweep_id: 'sweep', configuration_id: 'configuration', turn_id: 'turn', attempt_id: 'attempt', job_id: 'job' } as const;
const terminal = new Set(['succeeded', 'failed', 'cancelled', 'interrupted', 'skipped']);
const forbidden = new Set(['text', 'transcript', 'reply', 'audio', 'audio_uri', 'uri', 'path', 'model_path', 'prompt', 'dictionary', 'error', 'approved_text', 'expected_output']);

// UUIDs are identity only, never credentials. Native runtimes additionally use
// their own UUID runtime identity; tests inject deterministic IDs.
function uuid() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => {
    const n = Math.floor(Math.random() * 16); return (c === 'x' ? n : (n & 3) | 8).toString(16);
  });
}
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).filter(([, v]) => v !== undefined).map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(',')}}`;
  return JSON.stringify(value) ?? 'null';
}
function privateContent(value: unknown) {
  if (!value || typeof value !== 'object') return;
  for (const [key, item] of Object.entries(value)) {
    if (forbidden.has(key)) throw new Error(`Private content cannot be stored in observations (${key}).`);
    privateContent(item);
  }
}
function freeze<T>(value: T): T {
  if (value !== null && typeof value === 'object') {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}

/** Immutable archive and pending delivery are committed in one SQLite transaction. */
export function createBenchmarkStore(db: MetricsConnection, options: Options) {
  const now = options.now ?? Date.now, newID = options.uuid ?? uuid;
  const listeners = new Set<() => void>();
  let version = 0;
  let device: PhoneDevice = { installation_id: options.installationID, hardware: 'unknown', os_version: 'unknown', app_version: 'unknown', build: 'unknown' };
  db.execSync(`CREATE TABLE IF NOT EXISTS phone_observation_archive (
    record_id TEXT PRIMARY KEY NOT NULL, stream_id TEXT NOT NULL, sequence INTEGER NOT NULL,
    kind TEXT NOT NULL, entity_id TEXT NOT NULL, revision INTEGER NOT NULL,
    payload TEXT NOT NULL, device TEXT NOT NULL, created_at INTEGER NOT NULL,
    UNIQUE(stream_id,sequence), UNIQUE(kind,entity_id,revision)
  );
  CREATE TABLE IF NOT EXISTS phone_observation_pending (
    record_id TEXT PRIMARY KEY NOT NULL, blocked INTEGER NOT NULL DEFAULT 0, error TEXT
  );
  CREATE TABLE IF NOT EXISTS phone_observation_state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS phone_observation_pins (entity_id TEXT PRIMARY KEY NOT NULL);
  CREATE INDEX IF NOT EXISTS phone_observation_entities ON phone_observation_archive(kind,entity_id,revision);`);
  // Expression indexes also cover archives created before Results was added.
  // Scoped reads follow explicit parent links without parsing unrelated payloads in JS.
  for (const field of Object.keys(contextKinds)) db.execSync(`CREATE INDEX IF NOT EXISTS phone_results_${field} ON phone_observation_archive(json_extract(payload,'$.context.${field}'))`);
  db.runSync('INSERT OR IGNORE INTO phone_observation_state(key,value) VALUES(?,?)', 'stream', newID());
  db.runSync('INSERT OR IGNORE INTO phone_observation_state(key,value) VALUES(?,?)', 'sequence', '0');
  db.runSync('INSERT OR IGNORE INTO phone_observation_state(key,value) VALUES(?,?)', 'results_generation', '0');
  const savedDevice = db.getFirstSync<{ value: string }>("SELECT value FROM phone_observation_state WHERE key='device'");
  if (savedDevice) device = JSON.parse(savedDevice.value) as PhoneDevice;
  const publish = () => { version++; for (const listener of listeners) listener(); };
  function transaction<T>(work: () => T): T {
    db.execSync('BEGIN IMMEDIATE');
    let result: T;
    try {
      result = work();
      db.execSync("UPDATE phone_observation_state SET value=CAST(value AS INTEGER)+1 WHERE key='results_generation'");
      db.execSync('COMMIT');
    }
    catch (error) { db.execSync('ROLLBACK'); throw error; }
    publish(); return result;
  }
  function latest(kind: ObservationKind, id: string): PhoneObservation | null {
    const row = db.getFirstSync<{ payload: string }>('SELECT payload FROM phone_observation_archive WHERE kind=? AND entity_id=? ORDER BY revision DESC LIMIT 1', kind, id);
    return row ? JSON.parse(row.payload) as PhoneObservation : null;
  }
  function write(kind: ObservationKind, id: string, context: PhoneObservationContext, data: Data): PhoneObservation {
    privateContent(data);
    const previous = latest(kind, id);
    if (kind === 'configuration' && previous && canonical(previous.data) !== canonical(data)) throw new Error('A configuration identity cannot change.');
    if (previous && terminal.has(String(previous.data.status)) && data.status && data.status !== previous.data.status) throw new Error('A finished observation cannot restart.');
    const sequence = Number(db.getFirstSync<{ value: string }>("SELECT value FROM phone_observation_state WHERE key='sequence'")!.value) + 1;
    const record: PhoneObservation = { schema_version: 1, record_id: newID(), producer_stream_id: db.getFirstSync<{ value: string }>("SELECT value FROM phone_observation_state WHERE key='stream'")!.value,
      record_sequence: sequence, timestamp_ms: now(), kind, entity_id: id, revision: (previous?.revision ?? 0) + 1,
      context: { ...previous?.context, ...context }, data: { ...previous?.data, ...data } };
    db.runSync('INSERT INTO phone_observation_archive VALUES(?,?,?,?,?,?,?,?,?)', record.record_id, record.producer_stream_id, sequence, kind, id, record.revision, JSON.stringify(record), JSON.stringify(device), record.timestamp_ms);
    db.runSync('INSERT INTO phone_observation_pending(record_id) VALUES(?)', record.record_id);
    db.runSync("UPDATE phone_observation_state SET value=? WHERE key='sequence'", String(sequence));
    const size = db.getFirstSync<{ count: number; bytes: number }>('SELECT COUNT(*) AS count,COALESCE(SUM(length(CAST(payload AS BLOB))+length(CAST(device AS BLOB))),0) AS bytes FROM phone_observation_archive');
    if (size && (size.count > (options.maxRecords ?? 50000) || size.bytes > (options.maxBytes ?? 48 * 1024 * 1024))) throw Object.assign(new Error('Observation archive is full. Export and remove acknowledged, unpinned history before continuing.'), { code: 'archive_full' });
    return record;
  }
  function append(kind: ObservationKind, id: string, context: PhoneObservationContext, data: Data) { return transaction(() => write(kind, id, context, data)); }
  function register(kind: ObservationKind, context: PhoneObservationContext, data: Data = {}) {
    const id = newID();
    append(kind, id, context, { status: kind === 'conversation' ? 'active' : 'planned', ...data }); return id;
  }
  function complete(kind: ObservationKind, id: string, status: ExecutionStatus, data: Data = {}) {
    const previous = latest(kind, id);
    if (!previous) throw new Error(`Missing ${kind} registration.`);
    return append(kind, id, {}, { ...data, status, ...(terminal.has(status) ? { finished_at_ms: now() } : {}) });
  }
  function snapshot(): StoredPhoneObservation[] {
    return db.getAllSync<Row>('SELECT record_id,payload,device,created_at FROM phone_observation_archive ORDER BY sequence').map(row => ({ device: JSON.parse(row.device) as PhoneDevice, record: JSON.parse(row.payload) as PhoneObservation, received_at_ms: row.created_at }));
  }
  function entities() {
    return db.getAllSync<{ payload: string }>(`SELECT a.payload FROM phone_observation_archive a WHERE a.revision=(SELECT MAX(b.revision) FROM phone_observation_archive b WHERE b.kind=a.kind AND b.entity_id=a.entity_id)`).map(row => JSON.parse(row.payload) as PhoneObservation);
  }
  function readTransaction<T>(work: () => T): T {
    // A read transaction binds metadata and rows to the same SQLite snapshot;
    // it neither takes the writer lock nor emits a store change notification.
    db.execSync('BEGIN');
    try { const result = work(); db.execSync('COMMIT'); return result; }
    catch (error) { db.execSync('ROLLBACK'); throw error; }
  }
  function watermark() {
    const rows = db.getAllSync<{ key: string; value: string }>("SELECT key,value FROM phone_observation_state WHERE key IN ('stream','sequence','results_generation')");
    const values = Object.fromEntries(rows.map(row => [row.key, row.value]));
    return { stream: values.stream, sequence: Number(values.sequence), generation: Number(values.results_generation) };
  }
  function resultsHistory(options: ResultsHistoryOptions = {}) {
    const filters = { source: options.source, sweep_id: options.sweep_id, query: options.query?.trim() || undefined,
      since_ms: options.since_ms, until_ms: options.until_ms };
    const signature = canonical(filters);
    const limit = Math.min(50, Math.max(1, Number.isFinite(options.limit) ? Math.floor(options.limit!) : 20));
    type Cursor = { version: number; filters: string; sequence: number; rank: number; started_at_ms: number; entity_id: string };
    let cursor: Cursor | undefined;
    if (options.cursor) {
      try { cursor = JSON.parse(options.cursor) as Cursor; } catch { throw new Error('Invalid Results history cursor.'); }
      if (!cursor || cursor.version !== 1 || cursor.filters !== signature || !Number.isSafeInteger(cursor.sequence) || cursor.sequence < 0 ||
        ![0, 1].includes(cursor.rank) || !Number.isFinite(cursor.started_at_ms) || typeof cursor.entity_id !== 'string' || !cursor.entity_id) throw new Error('Invalid Results history cursor.');
    }
    return readTransaction(() => {
      const current = watermark();
      if (cursor && cursor.sequence > current.sequence) throw new Error('Invalid Results history cursor.');
      const sequence = cursor?.sequence ?? current.sequence;
      const clauses: string[] = [], params: (string | number | null)[] = [sequence, sequence, sequence];
      if (filters.source) { clauses.push('source=?'); params.push(filters.source); }
      if (filters.sweep_id) { clauses.push('sweep_id=?'); params.push(filters.sweep_id); }
      if (filters.query) { clauses.push("instr(lower(COALESCE(asr_name,'') || ' ' || COALESCE(reasoning_name,'') || ' ' || entity_id),lower(?))>0"); params.push(filters.query); }
      if (filters.since_ms !== undefined) { clauses.push('started_at_ms>=?'); params.push(filters.since_ms); }
      if (filters.until_ms !== undefined) { clauses.push('started_at_ms<=?'); params.push(filters.until_ms); }
      const cte = `WITH history AS (
        SELECT a.entity_id,json_extract(a.payload,'$.data.source') AS source,json_extract(a.payload,'$.data.status') AS status,
          COALESCE(json_extract(a.payload,'$.data.started_at_ms'),(SELECT MIN(created_at) FROM phone_observation_archive first WHERE first.kind=a.kind AND first.entity_id=a.entity_id)) AS started_at_ms,
          json_extract(a.payload,'$.context.sweep_id') AS sweep_id,json_extract(a.payload,'$.context.configuration_id') AS configuration_id,
          json_extract(c.payload,'$.data.asr.name') AS asr_name,json_extract(c.payload,'$.data.reasoning.name') AS reasoning_name,
          CASE WHEN json_extract(a.payload,'$.data.status') IN ('active','running') THEN 0 ELSE 1 END AS rank
        FROM phone_observation_archive a
        LEFT JOIN phone_observation_archive c ON c.kind='configuration' AND c.entity_id=json_extract(a.payload,'$.context.configuration_id')
          AND c.revision=(SELECT MAX(revision) FROM phone_observation_archive WHERE kind=c.kind AND entity_id=c.entity_id AND sequence<=?)
        WHERE a.kind='conversation' AND a.sequence<=? AND a.revision=(SELECT MAX(revision) FROM phone_observation_archive WHERE kind=a.kind AND entity_id=a.entity_id AND sequence<=?)
      ), filtered AS (SELECT * FROM history${clauses.length ? ` WHERE ${clauses.join(' AND ')}` : ''})`;
      const total = db.getFirstSync<{ count: number }>(`${cte} SELECT COUNT(*) AS count FROM filtered`, ...params)!.count;
      const pageParams = [...params];
      let after = '';
      if (cursor) {
        after = ' WHERE rank>? OR (rank=? AND (started_at_ms<? OR (started_at_ms=? AND entity_id<?)))';
        pageParams.push(cursor.rank, cursor.rank, cursor.started_at_ms, cursor.started_at_ms, cursor.entity_id);
      }
      const rows = db.getAllSync<ResultsHistoryItem & { rank: number }>(`${cte}
        SELECT entity_id,source,status,started_at_ms,configuration_id,asr_name,reasoning_name,rank FROM filtered${after}
        ORDER BY rank,started_at_ms DESC,entity_id DESC LIMIT ?`, ...pageParams, limit + 1);
      const visible = rows.slice(0, limit), last = visible.at(-1);
      const next_cursor = rows.length > limit && last ? JSON.stringify({ version: 1, filters: signature, sequence,
        rank: last.rank, started_at_ms: last.started_at_ms, entity_id: last.entity_id } satisfies Cursor) : null;
      return freeze({ items: visible.map(({ rank: _rank, ...item }) => item), total, next_cursor });
    });
  }
  function resultsSnapshot(scope: ResultsScope): ResultsSnapshot {
    const filters = Object.entries(contextKinds).filter(([field]) => field in scope && !!scope[field as keyof ResultsScope])
      .map(([field, kind]) => ({ field, kind, id: scope[field as keyof ResultsScope]! }));
    return readTransaction(() => {
      const { stream, sequence, generation } = watermark();
      const identity = { conversation_id: scope.conversation_id, sweep_id: scope.sweep_id, configuration_id: scope.configuration_id };
      const as_of = `local:${encodeURIComponent(stream)}:${sequence}:${generation}:${encodeURIComponent(canonical(identity))}`;
      // Empty selection is deliberately empty; Results never silently loads the archive.
      if (!filters.length) return freeze({ records: [], selected_record_keys: [], as_of, generation, sequence });
      const params: string[] = [];
      const descendants = filters.map(({ kind, id }, index) => {
        params.push(kind, id);
        const name = `scope_${index}`;
        return `${name}(kind,entity_id) AS (
          SELECT ?,?
          ${Object.entries(contextKinds).map(([field, parentKind]) => `UNION SELECT child.kind,child.entity_id FROM ${name} parent
            JOIN phone_observation_archive child ON json_extract(child.payload,'$.context.${field}')=parent.entity_id WHERE parent.kind='${parentKind}'`).join('\n')}
          UNION SELECT child.kind,child.entity_id FROM ${name} parent JOIN phone_observation_archive child
            ON child.kind='finalization' AND child.entity_id=parent.entity_id WHERE parent.kind IN ('conversation','attempt','sweep')
        )`;
      });
      const selected = filters.map((_, index) => `SELECT kind,entity_id FROM scope_${index}`).join(' INTERSECT ');
      const parents = Object.entries(contextKinds).map(([field, kind]) => `UNION SELECT '${kind}',json_extract(child.payload,'$.context.${field}')
        FROM included wanted JOIN phone_observation_archive child ON child.kind=wanted.kind AND child.entity_id=wanted.entity_id
        WHERE json_extract(child.payload,'$.context.${field}') IS NOT NULL`).join('\n');
      const rows = db.getAllSync<Row & { selected: number }>(`WITH RECURSIVE ${descendants.join(',')},
        selected(kind,entity_id) AS (${selected}), included(kind,entity_id) AS (SELECT kind,entity_id FROM selected ${parents})
        SELECT a.record_id,a.payload,a.device,a.created_at,EXISTS(SELECT 1 FROM selected s WHERE s.kind=a.kind AND s.entity_id=a.entity_id) AS selected
        FROM included i JOIN phone_observation_archive a ON a.kind=i.kind AND a.entity_id=i.entity_id ORDER BY a.sequence`, ...params);
      const records = rows.map(row => ({ device: JSON.parse(row.device) as PhoneDevice, record: JSON.parse(row.payload) as PhoneObservation, received_at_ms: row.created_at }));
      const selected_record_keys = records.flatMap((stored, index) => rows[index].selected ? [{ installation_id: stored.device.installation_id, record_id: stored.record.record_id }] : []);
      return freeze({ records, selected_record_keys, as_of, generation, sequence });
    });
  }
  const store = {
    newID, append, latest, snapshot, resultsHistory, resultsSnapshot,
    resultsStatus() {
      const row = db.getFirstSync<{ pending: number; blocked: number }>('SELECT COALESCE(SUM(1-blocked),0) AS pending,COALESCE(SUM(blocked),0) AS blocked FROM phone_observation_pending');
      return { pending: row?.pending ?? 0, blocked: row?.blocked ?? 0,
        lastUpload: Number(db.getFirstSync<{ value: string }>("SELECT value FROM phone_observation_state WHERE key='last_upload'")?.value) || null };
    },
    liveSummary() {
      // SQLite returns a few counters and eight attempts. Raw event payloads
      // never cross the JS bridge on the live screen's refresh timer.
      const counts = db.getAllSync<{ kind: string; count: number }>("SELECT kind,COUNT(DISTINCT entity_id) AS count FROM phone_observation_archive WHERE kind IN ('conversation','turn','attempt') GROUP BY kind");
      const events = db.getFirstSync<{ count: number }>("SELECT COALESCE(SUM(json_array_length(json_extract(payload,'$.data.batch.events'))),0) AS count FROM phone_observation_archive WHERE kind='metric_batch'");
      const attempts = db.getAllSync<{ payload: string }>("SELECT a.payload FROM phone_observation_archive a WHERE a.kind='attempt' AND a.revision=(SELECT MAX(b.revision) FROM phone_observation_archive b WHERE b.kind=a.kind AND b.entity_id=a.entity_id) ORDER BY a.sequence DESC LIMIT 8").map(row => JSON.parse(row.payload) as PhoneObservation);
      return { conversations: counts.find(row => row.kind === 'conversation')?.count ?? 0,
        turns: counts.find(row => row.kind === 'turn')?.count ?? 0,
        attempts: counts.find(row => row.kind === 'attempt')?.count ?? 0,
        metric_events: events?.count ?? 0, recent_attempts: attempts };
    },
    assertCanStart() {
      const size = db.getFirstSync<{ count: number; bytes: number }>('SELECT COUNT(*) AS count,COALESCE(SUM(length(CAST(payload AS BLOB))+length(CAST(device AS BLOB))),0) AS bytes FROM phone_observation_archive')!;
      if (size.count + (options.reserveRecords ?? 600) > (options.maxRecords ?? 50000) || size.bytes + (options.reserveBytes ?? 4 * 1024 * 1024) > (options.maxBytes ?? 48 * 1024 * 1024)) {
        throw Object.assign(new Error('Observation archive needs space for native metrics and finalization before another operation can start.'), { code: 'archive_full' });
      }
      if (device.hardware === 'unknown' || device.os_version === 'unknown') throw new Error('Resolve device metadata before starting an observed operation.');
    },
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    getVersion: () => version,
    setDevice(value: PhoneDevice) { if (value.installation_id !== options.installationID) throw new Error('Installation identity changed.'); device = value; db.runSync("INSERT OR REPLACE INTO phone_observation_state(key,value) VALUES('device',?)", JSON.stringify(value)); },
    device: () => ({ ...device }),
    registerConfiguration(data: Data) {
      const matching = entities().find(record => record.kind === 'configuration' && canonical(record.data) === canonical(data));
      if (matching) return matching.entity_id;
      const id = newID(); append('configuration', id, {}, data); return id;
    },
    registerConversation: (context: PhoneObservationContext, data: Data = {}) => register('conversation', context, { started_at_ms: now(), ...data }),
    registerTurn: (context: PhoneObservationContext, data: Data = {}) => register('turn', context, data),
    registerAttempt: (context: PhoneObservationContext, data: Data = {}) => register('attempt', context, data),
    registerSweep: (data: Data = {}) => register('sweep', {}, data),
    registerJob: (context: PhoneObservationContext, data: Data = {}) => register('job', context, data),
    completeAttempt: (id: string, status: ExecutionStatus, data?: Data) => complete('attempt', id, status, data),
    completeTurn: (id: string, status: ExecutionStatus, data?: Data) => complete('turn', id, status, data),
    completeConversation: (id: string, status: ExecutionStatus, data?: Data) => complete('conversation', id, status, data),
    completeSweep: (id: string, status: ExecutionStatus, data?: Data) => complete('sweep', id, status, data),
    completeJob: (id: string, status: ExecutionStatus, data?: Data) => complete('job', id, status, data),
    recordBatches(batches: { batch: MetricBatch; context: PhoneObservationContext }[]) {
      return transaction(() => batches.map(({ batch, context }) => {
        const identity = canonical([context.runtime_session_id ?? 'unknown', batch.run_id, batch.sequence_number]);
        const mapped = db.getFirstSync<{ value: string }>('SELECT value FROM phone_observation_state WHERE key=?', `batch:${identity}`);
        if (mapped) {
          const existing = latest('metric_batch', mapped.value)!;
          if (canonical(existing.data.batch) !== canonical(batch) || canonical(existing.context) !== canonical(context)) throw new Error('Metric batch identity has different content.');
          return existing;
        }
        const id = newID(); const record = write('metric_batch', id, context, { batch });
        db.runSync('INSERT INTO phone_observation_state(key,value) VALUES(?,?)', `batch:${identity}`, id); return record;
      }));
    },
    finalize(context: PhoneObservationContext, data: Data) {
      const kind = context.attempt_id ? 'attempt' : context.conversation_id ? 'conversation' : context.sweep_id ? 'sweep' : null;
      const id = context.attempt_id ?? context.conversation_id ?? context.sweep_id ?? newID();
      if (!kind) return append('finalization', id, context, data);
      const key = `${kind}_id` as keyof PhoneObservationContext;
      const required = snapshot().filter(item => item.record.kind !== 'finalization' && (item.record.entity_id === id || item.record.context[key] === id));
      const refs = required.map(item => item.record.record_id);
      const eventCount = required.filter(item => item.record.kind === 'metric_batch').reduce((n, item) => n + (item.record.data.batch as MetricBatch).events.length, 0);
      return append('finalization', id, context, { ...data, target_revision: latest(kind, id)?.revision ?? 0,
        required_record_ids: refs, record_count: refs.length, event_count: eventCount });
    },
    recoverInterrupted(reason = 'process_restart', runtimeID?: string) {
      const unfinished = entities().filter(record => ['attempt', 'turn', 'conversation', 'job', 'sweep'].includes(record.kind) && ['active', 'running'].includes(String(record.data.status)) && (!runtimeID || record.context.runtime_session_id !== runtimeID));
      if (!unfinished.length) return;
      transaction(() => {
        for (const record of unfinished) {
          if (record.kind === 'sweep' || record.kind === 'job') {
            write(record.kind, record.entity_id, {}, { status: 'running', pause_reason: reason, next_execution_required: true });
            continue;
          }
          write(record.kind, record.entity_id, {}, { status: 'interrupted', finished_at_ms: now(), reason_code: reason });
          if (record.kind === 'attempt' || record.kind === 'conversation') write('finalization', record.entity_id, { ...record.context, [`${record.kind}_id`]: record.entity_id }, { status: 'partial', event_count: 0, dropped_events: 0, reason_code: reason });
        }
      });
    },
    pending(limit = 50) {
      return db.getAllSync<Row>(`SELECT a.record_id,a.payload,a.device,a.created_at FROM phone_observation_archive a JOIN phone_observation_pending p ON p.record_id=a.record_id WHERE p.blocked=0 ORDER BY a.sequence LIMIT ?`, limit);
    },
    acknowledge(ids: string[]) {
      transaction(() => { for (const id of ids) db.runSync('DELETE FROM phone_observation_pending WHERE record_id=?', id); db.runSync("INSERT OR REPLACE INTO phone_observation_state(key,value) VALUES('last_upload',?)", String(now())); });
    },
    block(ids: string[], reason: string) { transaction(() => { for (const id of ids) db.runSync('UPDATE phone_observation_pending SET blocked=1,error=? WHERE record_id=?', reason, id); }); },
    retryBlocked() { transaction(() => db.runSync('UPDATE phone_observation_pending SET blocked=0,error=NULL')); },
    stats() {
      const row = db.getFirstSync<{ pending: number; blocked: number }>('SELECT COALESCE(SUM(1-blocked),0) AS pending,COALESCE(SUM(blocked),0) AS blocked FROM phone_observation_pending');
      const total = db.getFirstSync<{ count: number; bytes: number }>('SELECT COUNT(*) AS count,COALESCE(SUM(length(CAST(payload AS BLOB))+length(CAST(device AS BLOB))),0) AS bytes FROM phone_observation_archive');
      return { pending: row?.pending ?? 0, blocked: row?.blocked ?? 0, records: total?.count ?? 0, bytes: total?.bytes ?? 0, lastUpload: Number(db.getFirstSync<{ value: string }>("SELECT value FROM phone_observation_state WHERE key='last_upload'")?.value) || null };
    },
    pin(id: string, pinned: boolean) { transaction(() => { if (pinned) db.runSync('INSERT OR IGNORE INTO phone_observation_pins VALUES(?)', id); else db.runSync('DELETE FROM phone_observation_pins WHERE entity_id=?', id); }); },
    isPinned: (id: string) => !!db.getFirstSync('SELECT entity_id FROM phone_observation_pins WHERE entity_id=?', id),
    removeAcknowledgedHistory(id: string) {
      const records = snapshot().filter(({ record }) => record.entity_id === id || record.context.conversation_id === id || record.context.sweep_id === id);
      if (!records.length) return;
      if (records.some(({ record }) => store.isPinned(record.entity_id) || !!record.context.sweep_id && store.isPinned(record.context.sweep_id) || !!record.context.conversation_id && store.isPinned(record.context.conversation_id))) throw new Error('Unpin this history before removing it.');
      const latestRecords = entities().filter(record => records.some(item => item.record.entity_id === record.entity_id));
      if (latestRecords.some(record => ['active', 'running', 'planned'].includes(String(record.data.status)))) throw new Error('Finish this work before removing it.');
      transaction(() => {
        for (const { record } of records) {
          if (db.getFirstSync('SELECT record_id FROM phone_observation_pending WHERE record_id=?', record.record_id)) throw new Error('Pending or blocked observations cannot be removed.');
          db.runSync('DELETE FROM phone_observation_archive WHERE record_id=?', record.record_id);
        }
        // Remove deduplication keys only for records explicitly removed.
        db.execSync("DELETE FROM phone_observation_state WHERE key LIKE 'batch:%' AND value NOT IN (SELECT entity_id FROM phone_observation_archive WHERE kind='metric_batch')");
      });
    },
    exportJSON: () => JSON.stringify({ schema_version: 1, exported_at_ms: now(), records: snapshot() }, null, 2),
  };
  return store;
}
export type BenchmarkStore = ReturnType<typeof createBenchmarkStore>;
