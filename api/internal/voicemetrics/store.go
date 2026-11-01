package voicemetrics

import (
	"context"
	"database/sql"
	"encoding/json"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

// Store persists metrics produced by phone voice sessions independently from
// benchmark experiments. Phone activity is not a benchmark experiment.
type Store struct{ db *sql.DB }

func NewStore(db *sql.DB) *Store { return &Store{db: db} }

func (s *Store) Append(ctx context.Context, batch metrics.Batch) error {
	if err := batch.Validate(); err != nil {
		return err
	}
	payload, err := json.Marshal(batch)
	if err != nil {
		return err
	}
	_, err = s.db.ExecContext(ctx, `INSERT INTO voice_metric_batches(run_id,sequence_number,payload) VALUES(?,?,?) ON CONFLICT(run_id,sequence_number) DO NOTHING`, batch.RunID, batch.SequenceNumber, string(payload))
	return err
}

func (s *Store) List(ctx context.Context) ([]metrics.Batch, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT payload FROM voice_metric_batches ORDER BY run_id,sequence_number`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []metrics.Batch{}
	for rows.Next() {
		var payload string
		if err := rows.Scan(&payload); err != nil {
			return nil, err
		}
		var batch metrics.Batch
		if err := json.Unmarshal([]byte(payload), &batch); err != nil {
			return nil, err
		}
		result = append(result, batch)
	}
	return result, rows.Err()
}
