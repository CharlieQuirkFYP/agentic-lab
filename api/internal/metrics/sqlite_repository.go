package metrics

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
)

var ErrBatchConflict = errors.New("metrics batch identity already has different content")

type SQLiteRepository struct{ db *sql.DB }

func NewSQLiteRepository(db *sql.DB) *SQLiteRepository { return &SQLiteRepository{db: db} }

func (r *SQLiteRepository) AppendBatch(ctx context.Context, batch Batch) error {
	if err := batch.Validate(); err != nil {
		return err
	}
	if batch.ExperimentID == "" {
		return ErrExperimentIDRequired
	}
	payload, err := json.Marshal(batch)
	if err != nil {
		return err
	}
	result, err := r.db.ExecContext(ctx, `INSERT INTO metric_batches(experiment_id,run_id,sequence_number,payload) VALUES(?,?,?,?) ON CONFLICT(experiment_id,run_id,sequence_number) DO NOTHING`, batch.ExperimentID, batch.RunID, batch.SequenceNumber, string(payload))
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil || count > 0 {
		return err
	}
	var stored string
	if err := r.db.QueryRowContext(ctx, `SELECT payload FROM metric_batches WHERE experiment_id=? AND run_id=? AND sequence_number=?`, batch.ExperimentID, batch.RunID, batch.SequenceNumber).Scan(&stored); err != nil {
		return err
	}
	if stored != string(payload) {
		return ErrBatchConflict
	}
	return nil
}

func (r *SQLiteRepository) read(ctx context.Context, condition string, id string) ([]Batch, error) {
	rows, err := r.db.QueryContext(ctx, `SELECT payload FROM metric_batches WHERE `+condition+`=? ORDER BY experiment_id,run_id,sequence_number`, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	batches := []Batch{}
	for rows.Next() {
		var payload string
		if err := rows.Scan(&payload); err != nil {
			return nil, err
		}
		var batch Batch
		if err := json.Unmarshal([]byte(payload), &batch); err != nil {
			return nil, err
		}
		batches = append(batches, batch)
	}
	return batches, rows.Err()
}

func (r *SQLiteRepository) GetByExperimentID(ctx context.Context, id string) ([]Batch, error) {
	if isBlank(id) {
		return nil, ErrExperimentIDRequired
	}
	return r.read(ctx, "experiment_id", id)
}

func (r *SQLiteRepository) GetBatches(ctx context.Context, id string) ([]Batch, error) {
	if isBlank(id) {
		return nil, ErrRunIDRequired
	}
	return r.read(ctx, "run_id", id)
}

func (r *SQLiteRepository) GetEvents(ctx context.Context, id string) ([]Event, error) {
	batches, err := r.GetBatches(ctx, id)
	if err != nil {
		return nil, err
	}
	events := []Event{}
	for _, b := range batches {
		events = append(events, b.Events...)
	}
	return events, nil
}
