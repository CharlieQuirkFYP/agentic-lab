package phonemetrics

import (
	"context"
	"database/sql"
	"encoding/json"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

type Store struct{ db *sql.DB }

func NewStore(db *sql.DB) *Store { return &Store{db: db} }

// Append registers an observation-only experiment and its batch atomically.
// No benchmark job or inference request is created by a phone upload.
func (s *Store) Append(ctx context.Context, upload Upload) (string, error) {
	if err := upload.Validate(); err != nil {
		return "", err
	}
	id := upload.ExperimentID()
	batch := upload.Batch
	batch.ExperimentID = id
	batch.Events = append([]metrics.Event(nil), batch.Events...)
	for i := range batch.Events {
		batch.Events[i].ExperimentID = id
	}
	payload, err := json.Marshal(batch)
	if err != nil {
		return "", err
	}
	now := time.Now().UTC()
	experiment := benchmark.Experiment{
		ID: id, ResultSource: Source, UseCase: benchmark.UseCaseIncidentReporting,
		Dataset: "phone", Status: benchmark.ExperimentStatusCompleted,
		CreatedAt: now, Device: &benchmark.DeviceProfile{
			InstallationID: upload.Device.InstallationID, Hardware: upload.Device.Hardware,
			OSVersion: upload.Device.OSVersion, AppVersion: upload.Device.AppVersion,
			Build: upload.Device.Build, ExecutionLocation: "device",
		},
		ModelConfig:        benchmark.ModelConfig{Name: "Pheme on-device", Runtime: "native-ios"},
		EnvironmentProfile: benchmark.EnvironmentProfile{Name: upload.Device.Hardware + " / iOS " + upload.Device.OSVersion, Type: "ios-device"},
	}
	experimentJSON, err := json.Marshal(experiment)
	if err != nil {
		return "", err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return "", err
	}
	defer func() { _ = tx.Rollback() }()
	_, err = tx.ExecContext(ctx, `INSERT INTO experiments(id,status,use_case,dataset,model,created_at,payload)
		VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING`, id, experiment.Status,
		experiment.UseCase, experiment.Dataset, experiment.ModelConfig.Name,
		now.Format("2006-01-02T15:04:05.000000000Z"), string(experimentJSON))
	if err != nil {
		return "", err
	}
	var existingJSON string
	if err := tx.QueryRowContext(ctx, `SELECT payload FROM experiments WHERE id=?`, id).Scan(&existingJSON); err != nil {
		return "", err
	}
	var existing benchmark.Experiment
	if err := json.Unmarshal([]byte(existingJSON), &existing); err != nil {
		return "", err
	}
	if existing.ResultSource != Source || existing.Device == nil || *existing.Device != *experiment.Device {
		return "", metrics.ErrBatchConflict
	}
	result, err := tx.ExecContext(ctx, `INSERT INTO metric_batches(experiment_id,run_id,sequence_number,payload)
		VALUES(?,?,?,?) ON CONFLICT(experiment_id,run_id,sequence_number) DO NOTHING`,
		id, batch.RunID, batch.SequenceNumber, string(payload))
	if err != nil {
		return "", err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return "", err
	}
	if count == 0 {
		var stored string
		if err := tx.QueryRowContext(ctx, `SELECT payload FROM metric_batches WHERE experiment_id=? AND run_id=? AND sequence_number=?`,
			id, batch.RunID, batch.SequenceNumber).Scan(&stored); err != nil {
			return "", err
		}
		if stored != string(payload) {
			return "", metrics.ErrBatchConflict
		}
	}
	return id, tx.Commit()
}

func (s *Store) List(ctx context.Context, limit, offset int) (Page, error) {
	page := Page{Source: Source, Batches: []metrics.Batch{}, Limit: limit, Offset: offset}
	if limit < 1 || limit > 100 || offset < 0 {
		return page, benchmark.ErrInvalidQuery
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return page, err
	}
	defer func() { _ = tx.Rollback() }()
	const join = ` FROM metric_batches m JOIN experiments e ON e.id=m.experiment_id WHERE json_extract(e.payload,'$.result_source')=?`
	if err := tx.QueryRowContext(ctx, `SELECT COUNT(*)`+join, Source).Scan(&page.Total); err != nil {
		return page, err
	}
	rows, err := tx.QueryContext(ctx, `SELECT m.payload`+join+` ORDER BY e.created_at DESC,e.id,m.sequence_number LIMIT ? OFFSET ?`, Source, limit, offset)
	if err != nil {
		return page, err
	}
	defer rows.Close()
	for rows.Next() {
		var raw string
		if err := rows.Scan(&raw); err != nil {
			return page, err
		}
		var batch metrics.Batch
		if err := json.Unmarshal([]byte(raw), &batch); err != nil {
			return page, err
		}
		page.Batches = append(page.Batches, batch)
	}
	if err := rows.Err(); err != nil {
		return page, err
	}
	if err := rows.Close(); err != nil {
		return page, err
	}
	return page, tx.Commit()
}
