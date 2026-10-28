import { Directory, File, Paths } from "expo-file-system";
import { openDatabaseSync } from "expo-sqlite";

export const MAX_RECORDING_SECONDS = 10 * 60;
export const MAX_RECORDING_BYTES = 19_200_044;
const MAX_LOCAL_RECORDINGS = 5;
const MAX_LOCAL_BYTES = 50 * 1024 * 1024;

const directory = new Directory(Paths.document, "voice-recordings");
const legacyManifest = new File(directory, "manifest.json");
const db = openDatabaseSync("voice-recordings.db");

export type PendingRecording = {
  id: string; uri: string; bytes: number; durationSeconds: number;
  createdAt: number; updatedAt: number; idempotencyKey: string;
  status: "saved" | "uploading" | "failed"; turnId?: string; error?: string;
};

type RecordingRow = {
  id: string; uri: string; bytes: number; duration_seconds: number;
  created_at: number; updated_at: number; idempotency_key: string;
  status: PendingRecording["status"]; turn_id: string | null; error: string | null;
};

db.execSync(`CREATE TABLE IF NOT EXISTS recordings (
  id TEXT PRIMARY KEY NOT NULL, uri TEXT NOT NULL, bytes INTEGER NOT NULL,
  duration_seconds REAL NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  idempotency_key TEXT NOT NULL UNIQUE, status TEXT NOT NULL, turn_id TEXT, error TEXT
)`);

function ensureDirectory() { directory.create({ intermediates: true, idempotent: true }); }

function fromRow(row: RecordingRow): PendingRecording {
  return { id: row.id, uri: row.uri, bytes: row.bytes, durationSeconds: row.duration_seconds, createdAt: row.created_at, updatedAt: row.updated_at, idempotencyKey: row.idempotency_key, status: row.status, turnId: row.turn_id ?? undefined, error: row.error ?? undefined };
}

function rows() {
  return db.getAllSync<RecordingRow>("SELECT * FROM recordings ORDER BY updated_at DESC");
}

function migrateLegacyManifest() {
  if (!legacyManifest.exists || rows().length) return;
  try {
    const value = JSON.parse(legacyManifest.textSync()) as PendingRecording | PendingRecording[];
    const recordings = Array.isArray(value) ? value : value?.uri ? [value] : [];
    for (const recording of recordings) {
      db.runSync("INSERT OR IGNORE INTO recordings (id, uri, bytes, duration_seconds, created_at, updated_at, idempotency_key, status, turn_id, error) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", recording.id, recording.uri, recording.bytes, recording.durationSeconds, recording.createdAt, recording.updatedAt, recording.idempotencyKey, recording.status, recording.turnId ?? null, recording.error ?? null);
    }
    legacyManifest.delete();
  } catch { /* A corrupt legacy manifest must not prevent new recordings. */ }
}

migrateLegacyManifest();

function removeFile(recording: PendingRecording) {
  const file = new File(recording.uri);
  if (file.exists) file.delete();
}

/** Evict least-recently-used recordings until both local limits hold. */
export function purgeLeastRecentlyUsedRecordings(protectedIds: readonly string[] = []) {
  const protectedSet = new Set(protectedIds);
  const recordings = rows().map(fromRow).filter((recording) => new File(recording.uri).exists);
  let totalBytes = recordings.reduce((total, recording) => total + recording.bytes, 0);
  const candidates = recordings.filter((recording) => !protectedSet.has(recording.id)).sort((left, right) => left.updatedAt - right.updatedAt);
  const remove = new Set<string>();
  while ((recordings.length - remove.size > MAX_LOCAL_RECORDINGS || totalBytes > MAX_LOCAL_BYTES) && candidates.length) {
    const oldest = candidates.shift();
    if (!oldest) break;
    remove.add(oldest.id); totalBytes -= oldest.bytes; removeFile(oldest);
  }
  for (const id of remove) db.runSync("DELETE FROM recordings WHERE id = ?", id);
  for (const recording of rows().map(fromRow)) if (!new File(recording.uri).exists) db.runSync("DELETE FROM recordings WHERE id = ?", recording.id);
}

export function saveRecording(wav: Uint8Array, durationSeconds: number): PendingRecording {
  if (durationSeconds > MAX_RECORDING_SECONDS) throw new Error("Recordings are limited to 10 minutes.");
  if (wav.byteLength > MAX_RECORDING_BYTES) throw new Error("This recording is too large. Keep voice reports under 10 minutes.");
  ensureDirectory();
  const id = `recording-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const file = new File(directory, `${id}.wav`); file.write(wav);
  const now = Date.now();
  const value: PendingRecording = { id, uri: file.uri, bytes: wav.byteLength, durationSeconds, createdAt: now, updatedAt: now, idempotencyKey: `ios-${id}`, status: "saved" };
  db.runSync("INSERT INTO recordings (id, uri, bytes, duration_seconds, created_at, updated_at, idempotency_key, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", value.id, value.uri, value.bytes, value.durationSeconds, value.createdAt, value.updatedAt, value.idempotencyKey, value.status);
  purgeLeastRecentlyUsedRecordings([value.id]);
  return value;
}

export function readPendingRecording(): PendingRecording | null {
  return rows().map(fromRow).find((recording) => new File(recording.uri).exists) ?? null;
}

export function updatePendingRecording(patch: Partial<PendingRecording>) {
  const current = readPendingRecording(); if (!current) return;
  const next = { ...current, ...patch, updatedAt: Date.now() };
  db.runSync("UPDATE recordings SET updated_at = ?, status = ?, turn_id = ?, error = ? WHERE id = ?", next.updatedAt, next.status, next.turnId ?? null, next.error ?? null, current.id);
  purgeLeastRecentlyUsedRecordings([current.id]);
}

export function removePendingRecording() {
  const current = readPendingRecording(); if (!current) return;
  removeFile(current); db.runSync("DELETE FROM recordings WHERE id = ?", current.id);
}

export function purgeExpiredRecording() { purgeLeastRecentlyUsedRecordings(); }

export function pendingFile(recording: PendingRecording) { return new File(recording.uri); }
