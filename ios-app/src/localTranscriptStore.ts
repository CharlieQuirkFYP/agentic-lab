import { openDatabaseSync } from 'expo-sqlite';
import type { LocalResult } from '../modules/pheme-native/src/PhemeNative.types';
import { initializeLocalTranscriptsSchema } from './localStorageSchema';

const db = openDatabaseSync('voice-recordings.db');
initializeLocalTranscriptsSchema(db);

// Transcript recovery only. Metric-batch persistence/acknowledged upload is a
// later milestone; retaining a UI observation is not a durable telemetry outbox.
export function saveLocalTranscript(result: LocalResult) {
  db.runSync('INSERT INTO local_transcriptions(run_id,payload,draft,updated_at) VALUES(?,?,?,?) ON CONFLICT(run_id) DO NOTHING',
    result.transcription.run_id, JSON.stringify(result), result.transcription.result.text, Date.now());
}
export function updateLocalDraft(runID: string, draft: string) {
  db.runSync('UPDATE local_transcriptions SET draft=?,updated_at=? WHERE run_id=?', draft, Date.now(), runID);
}
export function latestLocalTranscript(): { result: LocalResult; draft: string } | null {
  const row = db.getFirstSync<{ payload: string; draft: string }>('SELECT payload,draft FROM local_transcriptions ORDER BY updated_at DESC LIMIT 1');
  if (!row) return null;
  return { result: JSON.parse(row.payload) as LocalResult, draft: row.draft };
}
