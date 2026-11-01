package voicehistory

import (
	"context"
	"database/sql"
	"encoding/json"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

type Session struct {
	ID           string    `json:"id"`
	Status       string    `json:"status"`
	Transcript   string    `json:"transcript"`
	ApprovedText string    `json:"approved_text,omitempty"`
	Reply        string    `json:"reply"`
	Error        string    `json:"error,omitempty"`
	CreatedAt    time.Time `json:"created_at"`
	UpdatedAt    time.Time `json:"updated_at"`
}

type Store struct{ db *sql.DB }

func NewStore(db *sql.DB) *Store {
	_, _ = db.Exec(`CREATE TABLE IF NOT EXISTS voice_sessions (id TEXT PRIMARY KEY, status TEXT NOT NULL, transcript TEXT NOT NULL, approved_text TEXT NOT NULL, reply TEXT NOT NULL, error TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL); CREATE INDEX IF NOT EXISTS voice_sessions_updated ON voice_sessions(updated_at DESC, id DESC);`)
	return &Store{db: db}
}

func (s *Store) Apply(ctx context.Context, fallbackID string, event pheme.SSEEvent) {
	if s == nil || s.db == nil {
		return
	}
	id := fallbackID
	if value, ok := event.Data["turn_id"].(string); ok && value != "" {
		id = value
	}
	if id == "" {
		return
	}
	now := time.Now().UTC().Format(time.RFC3339Nano)
	var current Session
	var createdAt, updatedAt string
	err := s.db.QueryRowContext(ctx, `SELECT id,status,transcript,approved_text,reply,error,created_at,updated_at FROM voice_sessions WHERE id=?`, id).Scan(&current.ID, &current.Status, &current.Transcript, &current.ApprovedText, &current.Reply, &current.Error, &createdAt, &updatedAt)
	if err != nil {
		current = Session{ID: id, Status: "transcribing", CreatedAt: time.Now().UTC()}
	} else {
		current.CreatedAt, _ = time.Parse(time.RFC3339Nano, createdAt)
		current.UpdatedAt, _ = time.Parse(time.RFC3339Nano, updatedAt)
	}
	current.UpdatedAt = time.Now().UTC()
	switch event.Event {
	case "turn.snapshot":
		current.Status = stringValue(event.Data["status"])
		current.Transcript = stringValue(event.Data["transcript"])
		current.ApprovedText = stringValue(event.Data["approved_text"])
		current.Reply = stringValue(event.Data["reply"])
		if raw, ok := event.Data["error"]; ok && raw != nil {
			encoded, _ := json.Marshal(raw)
			current.Error = string(encoded)
		}
	case "transcript.ready":
		current.Status = "awaiting_review"
		current.Transcript = stringValue(event.Data["text"])
	case "question.approved":
		current.Status = "generating"
		current.ApprovedText = stringValue(event.Data["text"])
	case "reply.started":
		current.Status = "generating"
	case "reply.delta":
		current.Status = "generating"
		current.Reply += stringValue(event.Data["text"])
	case "reply.completed":
		current.Status = "completed"
		if text := stringValue(event.Data["text"]); text != "" {
			current.Reply = text
		}
	case "turn.failed", "reply.failed":
		current.Status = "failed"
		if raw, ok := event.Data["error"]; ok {
			encoded, _ := json.Marshal(raw)
			current.Error = string(encoded)
		}
	case "turn.cancelled":
		current.Status = "cancelled"
	default:
		return
	}
	_, _ = s.db.ExecContext(ctx, `INSERT INTO voice_sessions(id,status,transcript,approved_text,reply,error,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,transcript=excluded.transcript,approved_text=excluded.approved_text,reply=excluded.reply,error=excluded.error,updated_at=excluded.updated_at`, current.ID, current.Status, current.Transcript, current.ApprovedText, current.Reply, current.Error, current.CreatedAt.Format(time.RFC3339Nano), now)
}

func stringValue(value any) string { text, _ := value.(string); return text }

func (s *Store) List(ctx context.Context) ([]Session, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT id,status,transcript,approved_text,reply,error,created_at,updated_at FROM voice_sessions ORDER BY updated_at DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	sessions := []Session{}
	for rows.Next() {
		var item Session
		var createdAt, updatedAt string
		if err := rows.Scan(&item.ID, &item.Status, &item.Transcript, &item.ApprovedText, &item.Reply, &item.Error, &createdAt, &updatedAt); err != nil {
			return nil, err
		}
		item.CreatedAt, _ = time.Parse(time.RFC3339Nano, createdAt)
		item.UpdatedAt, _ = time.Parse(time.RFC3339Nano, updatedAt)
		sessions = append(sessions, item)
	}
	return sessions, rows.Err()
}
