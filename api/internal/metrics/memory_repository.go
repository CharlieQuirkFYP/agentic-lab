package metrics

import (
	"context"
	"sync"
)

// MemoryRepository is a thread-safe in-memory metrics store for local
// development, tests, and the initial dashboard implementation.
type MemoryRepository struct {
	mu      sync.RWMutex
	batches []Batch
}

// InMemoryRepository is a descriptive alias for MemoryRepository.
type InMemoryRepository = MemoryRepository

func NewMemoryRepository() *MemoryRepository {
	return &MemoryRepository{}
}

// NewInMemoryRepository is an explicit constructor alias.
func NewInMemoryRepository() *InMemoryRepository {
	return NewMemoryRepository()
}

func (r *MemoryRepository) AppendBatch(ctx context.Context, batch Batch) error {
	if err := contextError(ctx); err != nil {
		return err
	}
	if r == nil {
		return ErrMetricsRepositoryFailure
	}
	if err := batch.Validate(); err != nil {
		return err
	}

	r.mu.Lock()
	defer r.mu.Unlock()
	if err := contextError(ctx); err != nil {
		return err
	}
	r.batches = append(r.batches, cloneBatch(batch))
	return nil
}

// Append is a concise method name for callers using the repository directly.
func (r *MemoryRepository) Append(ctx context.Context, batch Batch) error {
	return r.AppendBatch(ctx, batch)
}

// GetBatches returns all batches belonging to a run in append order.
func (r *MemoryRepository) GetBatches(ctx context.Context, runID string) ([]Batch, error) {
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	if isBlank(runID) {
		return nil, ErrRunIDRequired
	}
	if r == nil {
		return nil, ErrMetricsRepositoryFailure
	}

	r.mu.RLock()
	defer r.mu.RUnlock()
	if err := contextError(ctx); err != nil {
		return nil, err
	}

	result := make([]Batch, 0)
	for _, batch := range r.batches {
		if batch.RunID == runID {
			result = append(result, cloneBatch(batch))
		}
	}
	return result, nil
}

// GetByRunID is an explicit retrieval name for future handlers.
func (r *MemoryRepository) GetByRunID(ctx context.Context, runID string) ([]Batch, error) {
	return r.GetBatches(ctx, runID)
}

// GetEvents returns a copy of the events belonging to a run, flattened in
// batch append order. Events remain separate from benchmark results.
func (r *MemoryRepository) GetEvents(ctx context.Context, runID string) ([]Event, error) {
	batches, err := r.GetBatches(ctx, runID)
	if err != nil {
		return nil, err
	}

	events := make([]Event, 0)
	for _, batch := range batches {
		events = append(events, batch.Events...)
	}
	return events, nil
}

// GetByExperimentID returns copies of batches associated with an experiment.
// The envelope identifier is preferred, while event identifiers are also
// considered for batches that intentionally omit the optional envelope value.
func (r *MemoryRepository) GetByExperimentID(ctx context.Context, experimentID string) ([]Batch, error) {
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	if isBlank(experimentID) {
		return nil, ErrExperimentIDRequired
	}
	if r == nil {
		return nil, ErrMetricsRepositoryFailure
	}

	r.mu.RLock()
	defer r.mu.RUnlock()
	if err := contextError(ctx); err != nil {
		return nil, err
	}

	result := make([]Batch, 0)
	for _, batch := range r.batches {
		if batchExperimentMatches(batch, experimentID) {
			result = append(result, cloneBatch(batch))
		}
	}
	return result, nil
}

// ListByExperimentID is an alternate collection-oriented name.
func (r *MemoryRepository) ListByExperimentID(ctx context.Context, experimentID string) ([]Batch, error) {
	return r.GetByExperimentID(ctx, experimentID)
}

// GetBatch retrieves one batch by its run-local batch sequence number.
func (r *MemoryRepository) GetBatch(ctx context.Context, runID string, sequenceNumber int64) (Batch, error) {
	batches, err := r.GetBatches(ctx, runID)
	if err != nil {
		return Batch{}, err
	}
	for _, batch := range batches {
		if batch.SequenceNumber == sequenceNumber {
			return batch, nil
		}
	}
	return Batch{}, ErrMetricsBatchNotFound
}

func batchExperimentMatches(batch Batch, experimentID string) bool {
	if batch.ExperimentID == experimentID {
		return true
	}
	for _, event := range batch.Events {
		if event.ExperimentID == experimentID {
			return true
		}
	}
	return false
}

func cloneBatch(batch Batch) Batch {
	clone := batch
	if batch.Events != nil {
		clone.Events = make([]Event, len(batch.Events))
		copy(clone.Events, batch.Events)
	}
	return clone
}

func contextError(ctx context.Context) error {
	if ctx == nil {
		return nil
	}
	return ctx.Err()
}
