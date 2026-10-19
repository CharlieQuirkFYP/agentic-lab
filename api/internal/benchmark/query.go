package benchmark

import (
	"context"
	"errors"
	"sort"
	"time"
)

var ErrInvalidQuery = errors.New("invalid experiment query")

type ListQuery struct {
	Limit         int
	Offset        int
	Status        ExperimentStatus
	UseCase       UseCase
	Dataset       string
	Model         string
	CreatedAfter  *time.Time
	CreatedBefore *time.Time
}

type ExperimentPage struct {
	Experiments []Experiment `json:"experiments"`
	Total       int          `json:"total"`
	Limit       int          `json:"limit"`
	Offset      int          `json:"offset"`
}

func (q ListQuery) Validate() error {
	if q.Limit < 1 || q.Limit > 100 || q.Offset < 0 {
		return ErrInvalidQuery
	}
	if q.Status != "" && q.Status != ExperimentStatusQueued && q.Status != ExperimentStatusRunning && q.Status != ExperimentStatusCompleted && q.Status != ExperimentStatusFailed {
		return ErrInvalidQuery
	}
	if q.UseCase != "" && !q.UseCase.IsSupported() {
		return ErrInvalidQuery
	}
	if q.CreatedAfter != nil && q.CreatedBefore != nil && q.CreatedAfter.After(*q.CreatedBefore) {
		return ErrInvalidQuery
	}
	return nil
}

func (s *Service) ListExperiments(ctx context.Context, q ListQuery) (ExperimentPage, error) {
	if err := q.Validate(); err != nil {
		return ExperimentPage{}, err
	}
	return s.repository.List(ctx, q)
}

func (r *MemoryRepository) List(ctx context.Context, q ListQuery) (ExperimentPage, error) {
	if err := q.Validate(); err != nil {
		return ExperimentPage{}, err
	}
	if err := ctx.Err(); err != nil {
		return ExperimentPage{}, err
	}
	r.mu.RLock()
	defer r.mu.RUnlock()
	items := make([]Experiment, 0)
	for _, e := range r.experiments {
		if q.Status != "" && e.Status != q.Status || q.UseCase != "" && e.UseCase != q.UseCase || q.Dataset != "" && e.Dataset != q.Dataset || q.Model != "" && e.ModelConfig.Name != q.Model {
			continue
		}
		if q.CreatedAfter != nil && e.CreatedAt.Before(*q.CreatedAfter) || q.CreatedBefore != nil && e.CreatedAt.After(*q.CreatedBefore) {
			continue
		}
		items = append(items, cloneExperiment(e))
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].CreatedAt.Equal(items[j].CreatedAt) {
			return items[i].ID > items[j].ID
		}
		return items[i].CreatedAt.After(items[j].CreatedAt)
	})
	page := ExperimentPage{Experiments: []Experiment{}, Total: len(items), Limit: q.Limit, Offset: q.Offset}
	if q.Offset < len(items) {
		page.Experiments = items[q.Offset : q.Offset+min(q.Limit, len(items)-q.Offset)]
	}
	return page, nil
}
