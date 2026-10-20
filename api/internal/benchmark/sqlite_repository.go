package benchmark

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"
)

type SQLiteRepository struct{ db *sql.DB }

func NewSQLiteRepository(db *sql.DB) *SQLiteRepository { return &SQLiteRepository{db: db} }

// Fixed-width UTC timestamps preserve chronological order in SQLite text indexes.
func databaseTime(t time.Time) string { return t.UTC().Format("2006-01-02T15:04:05.000000000Z") }

func (r *SQLiteRepository) Create(ctx context.Context, e Experiment) error {
	payload, err := json.Marshal(e)
	if err != nil {
		return err
	}
	_, err = r.db.ExecContext(ctx, `INSERT INTO experiments(id,status,use_case,dataset,model,created_at,payload) VALUES(?,?,?,?,?,?,?)`, e.ID, e.Status, e.UseCase, e.Dataset, e.ModelConfig.Name, databaseTime(e.CreatedAt), string(payload))
	return err
}

func (r *SQLiteRepository) GetByID(ctx context.Context, id string) (Experiment, error) {
	var payload string
	err := r.db.QueryRowContext(ctx, `SELECT payload FROM experiments WHERE id=?`, id).Scan(&payload)
	if errors.Is(err, sql.ErrNoRows) {
		return Experiment{}, ErrExperimentNotFound
	}
	if err != nil {
		return Experiment{}, err
	}
	var e Experiment
	err = json.Unmarshal([]byte(payload), &e)
	return e, err
}

func (r *SQLiteRepository) Update(ctx context.Context, e Experiment) error {
	payload, err := json.Marshal(e)
	if err != nil {
		return err
	}
	result, err := r.db.ExecContext(ctx, `UPDATE experiments SET status=?,use_case=?,dataset=?,model=?,created_at=?,payload=? WHERE id=?`, e.Status, e.UseCase, e.Dataset, e.ModelConfig.Name, databaseTime(e.CreatedAt), string(payload), e.ID)
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err == nil && count == 0 {
		return ErrExperimentNotFound
	}
	return err
}

func (r *SQLiteRepository) List(ctx context.Context, q ListQuery) (ExperimentPage, error) {
	if err := q.Validate(); err != nil {
		return ExperimentPage{}, err
	}
	clauses := []string{"1=1"}
	args := []any{}
	for _, filter := range []struct {
		column string
		value  string
	}{{"status", string(q.Status)}, {"use_case", string(q.UseCase)}, {"dataset", q.Dataset}, {"model", q.Model}} {
		if filter.value != "" {
			clauses = append(clauses, filter.column+"=?")
			args = append(args, filter.value)
		}
	}
	if q.CreatedAfter != nil {
		clauses = append(clauses, "created_at>=?")
		args = append(args, databaseTime(*q.CreatedAfter))
	}
	if q.CreatedBefore != nil {
		clauses = append(clauses, "created_at<=?")
		args = append(args, databaseTime(*q.CreatedBefore))
	}
	where := strings.Join(clauses, " AND ")
	page := ExperimentPage{Experiments: []Experiment{}, Limit: q.Limit, Offset: q.Offset}
	tx, err := r.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return page, err
	}
	defer func() { _ = tx.Rollback() }()
	if err := tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM experiments WHERE "+where, args...).Scan(&page.Total); err != nil {
		return page, err
	}
	args = append(args, q.Limit, q.Offset)
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM experiments WHERE "+where+" ORDER BY created_at DESC,id DESC LIMIT ? OFFSET ?", args...)
	if err != nil {
		return page, err
	}
	defer rows.Close()
	for rows.Next() {
		var payload string
		if err := rows.Scan(&payload); err != nil {
			return page, err
		}
		var e Experiment
		if err := json.Unmarshal([]byte(payload), &e); err != nil {
			return page, err
		}
		page.Experiments = append(page.Experiments, e)
	}
	if err := rows.Err(); err != nil {
		return page, err
	}
	if err := rows.Close(); err != nil {
		return page, err
	}
	return page, tx.Commit()
}

// RecoverInterrupted fails pending work on startup. The volatile queue is never
// replayed, avoiding accidental duplicate execution when the real runner arrives.
func (r *SQLiteRepository) RecoverInterrupted(ctx context.Context) error {
	for _, status := range []ExperimentStatus{ExperimentStatusQueued, ExperimentStatusRunning} {
		for {
			page, err := r.List(ctx, ListQuery{Limit: 100, Status: status})
			if err != nil {
				return err
			}
			if len(page.Experiments) == 0 {
				break
			}
			for _, e := range page.Experiments {
				now := time.Now().UTC()
				e.Status = ExperimentStatusFailed
				e.CompletedAt = &now
				e.Result = &Result{Success: false, ErrorMessage: "experiment interrupted by API restart; create a new experiment to retry"}
				if err := r.Update(ctx, e); err != nil {
					return err
				}
			}
		}
	}
	return nil
}
