import { openDatabaseSync } from "expo-sqlite";
import type { Turn } from "./types";

const db = openDatabaseSync("voice-recordings.db");

db.execSync(`CREATE TABLE IF NOT EXISTS chat_history (
  id TEXT PRIMARY KEY NOT NULL,
  status TEXT NOT NULL,
  transcript TEXT NOT NULL,
  approved_text TEXT,
  reply TEXT NOT NULL,
  error TEXT,
  updated_at INTEGER NOT NULL
)`);

type ChatRow = { id: string; status: Turn["status"]; transcript: string; approved_text: string | null; reply: string; error: string | null; updated_at: number };

export function saveChat(turn: Turn) {
  if (turn.id.startsWith("pending-")) return;
  db.runSync(
    "INSERT OR REPLACE INTO chat_history (id, status, transcript, approved_text, reply, error, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    turn.id, turn.status, turn.transcript, turn.approvedText, turn.reply, turn.error ?? null, Date.now(),
  );
}

export function listRecentChats(): Turn[] {
  return db.getAllSync<ChatRow>("SELECT * FROM chat_history ORDER BY updated_at DESC LIMIT 50").map((row) => ({
    id: row.id,
    status: row.status,
    transcript: row.transcript,
    approvedText: row.approved_text,
    reply: row.reply,
    error: row.error ?? undefined,
    attachments: [],
  }));
}

export function deleteChat(id: string) {
  db.runSync("DELETE FROM chat_history WHERE id = ?", id);
}
