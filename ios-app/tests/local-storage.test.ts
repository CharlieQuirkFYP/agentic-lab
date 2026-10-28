import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { initializeLocalTranscriptsSchema, initializeRecordingsSchema, type SchemaConnection } from '../src/localStorageSchema.ts';

test('migration preserves legacy pending recordings and separates local files', () => {
  const db = new DatabaseSync(':memory:');
  const adapter: SchemaConnection = { execSync: sql => db.exec(sql), getAllSync: <T>(sql: string) => db.prepare(sql).all() as T[] };
  try {
    db.exec(`CREATE TABLE recordings(id TEXT PRIMARY KEY NOT NULL,uri TEXT NOT NULL,bytes INTEGER NOT NULL,duration_seconds REAL NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,idempotency_key TEXT NOT NULL UNIQUE,status TEXT NOT NULL,turn_id TEXT,error TEXT);
      INSERT INTO recordings VALUES('old','file:///old.wav',44,1,1,1,'retry-old','failed',NULL,'offline');`);
    initializeRecordingsSchema(adapter); initializeRecordingsSchema(adapter);
    const old = db.prepare("SELECT purpose,idempotency_key,error FROM recordings WHERE id='old'").get();
    assert.equal(old?.purpose, 'remote'); assert.equal(old?.idempotency_key, 'retry-old'); assert.equal(old?.error, 'offline');
    db.exec("INSERT INTO recordings VALUES('local','file:///local.wav',44,1,2,2,'local-key','saved',NULL,NULL,'local')");
    assert.equal(db.prepare("SELECT id FROM recordings WHERE purpose='remote'").all().length, 1);
    assert.equal(db.prepare("SELECT id FROM recordings WHERE purpose='local'").get()?.id, 'local');
    initializeLocalTranscriptsSchema(adapter); initializeLocalTranscriptsSchema(adapter);
    db.prepare('INSERT INTO local_transcriptions VALUES(?,?,?,?)').run('run-1', '{"execution_location":"device"}', 'edited draft', 3);
    assert.equal(db.prepare('SELECT draft FROM local_transcriptions WHERE run_id=?').get('run-1')?.draft, 'edited draft');
  } finally { db.close(); }
});
