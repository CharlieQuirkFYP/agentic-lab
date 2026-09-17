package metrics

import "context"

// Repository is the persistence boundary for metric batches. It deliberately
// contains no benchmark Experiment or Result types.
type Repository interface {
	AppendBatch(ctx context.Context, batch Batch) error
}

// ReadRepository is the optional read side used by the service and the
// in-memory implementation. Keeping reads separate lets append-only storage
// implementations satisfy the smaller Repository contract.
type ReadRepository interface {
	Repository
	GetBatches(ctx context.Context, runID string) ([]Batch, error)
	GetEvents(ctx context.Context, runID string) ([]Event, error)
	GetByExperimentID(ctx context.Context, experimentID string) ([]Batch, error)
}

// MetricsRepository is a descriptive alias for Repository.
type MetricsRepository = Repository
