export interface SchemaConnection {
  execSync(sql: string): void;
  getAllSync<T>(sql: string): T[];
}

export function initializeRecordingsSchema(db: SchemaConnection) {
  db.execSync(`CREATE TABLE IF NOT EXISTS recordings (
    id TEXT PRIMARY KEY NOT NULL, uri TEXT NOT NULL, bytes INTEGER NOT NULL,
    duration_seconds REAL NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE, status TEXT NOT NULL, turn_id TEXT, error TEXT,
    purpose TEXT NOT NULL DEFAULT 'remote'
  )`);
  if (!db.getAllSync<{ name: string }>('PRAGMA table_info(recordings)').some(column => column.name === 'purpose')) {
    db.execSync("ALTER TABLE recordings ADD COLUMN purpose TEXT NOT NULL DEFAULT 'remote'");
  }
}

export function initializeLocalTranscriptsSchema(db: SchemaConnection) {
  db.execSync(`CREATE TABLE IF NOT EXISTS local_transcriptions (
    run_id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL, draft TEXT NOT NULL, updated_at INTEGER NOT NULL
  )`);
}
