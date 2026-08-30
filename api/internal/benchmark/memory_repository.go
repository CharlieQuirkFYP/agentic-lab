package benchmark

import (
	"context"
	"sync"
)

type MemoryRepository struct {
	mu          sync.RWMutex
	experiments map[string]Experiment
}

func NewMemoryRepository() *MemoryRepository {
	return &MemoryRepository{
		experiments: make(map[string]Experiment),
	}
}

func (r *MemoryRepository) Create(ctx context.Context, experiment Experiment) error {
	if err := ctx.Err(); err != nil {
		return err
	}

	r.mu.Lock()
	defer r.mu.Unlock()

	r.experiments[experiment.ID] = cloneExperiment(experiment)
	return nil
}

func (r *MemoryRepository) GetByID(ctx context.Context, id string) (Experiment, error) {
	if err := ctx.Err(); err != nil {
		return Experiment{}, err
	}

	r.mu.RLock()
	defer r.mu.RUnlock()

	experiment, ok := r.experiments[id]
	if !ok {
		return Experiment{}, ErrExperimentNotFound
	}

	return cloneExperiment(experiment), nil
}

func (r *MemoryRepository) Update(ctx context.Context, experiment Experiment) error {
	if err := ctx.Err(); err != nil {
		return err
	}

	r.mu.Lock()
	defer r.mu.Unlock()

	if _, ok := r.experiments[experiment.ID]; !ok {
		return ErrExperimentNotFound
	}

	r.experiments[experiment.ID] = cloneExperiment(experiment)
	return nil
}

func cloneExperiment(experiment Experiment) Experiment {
	clone := experiment

	if experiment.StartedAt != nil {
		startedAt := *experiment.StartedAt
		clone.StartedAt = &startedAt
	}
	if experiment.CompletedAt != nil {
		completedAt := *experiment.CompletedAt
		clone.CompletedAt = &completedAt
	}
	if experiment.Result != nil {
		result := *experiment.Result
		clone.Result = &result
	}

	return clone
}
