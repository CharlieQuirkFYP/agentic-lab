package metrics

import "context"

// Service validates metric batches before delegating persistence.
type Service struct {
	repository Repository
}

// MetricsService is a descriptive alias for Service.
type MetricsService = Service

func NewService(repository Repository) *Service {
	return &Service{repository: repository}
}

// NewMetricsService is an explicit constructor alias.
func NewMetricsService(repository Repository) *Service {
	return NewService(repository)
}

// Append validates and stores one batch.
func (s *Service) Append(ctx context.Context, batch Batch) error {
	if err := contextError(ctx); err != nil {
		return err
	}
	if s == nil || s.repository == nil {
		return ErrMetricsRepositoryFailure
	}
	if err := batch.Validate(); err != nil {
		return err
	}
	return s.repository.AppendBatch(ctx, batch)
}

// AppendBatch is the explicit batch-oriented name used by the repository API.
func (s *Service) AppendBatch(ctx context.Context, batch Batch) error {
	return s.Append(ctx, batch)
}

// Ingest is a concise alias for Append.
func (s *Service) Ingest(ctx context.Context, batch Batch) error {
	return s.Append(ctx, batch)
}

// GetByExperimentID retrieves metric batches without coupling callers to the
// concrete in-memory repository.
func (s *Service) GetByExperimentID(ctx context.Context, experimentID string) ([]Batch, error) {
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	if s == nil || s.repository == nil {
		return nil, ErrMetricsRepositoryFailure
	}
	reader, ok := s.repository.(interface {
		GetByExperimentID(context.Context, string) ([]Batch, error)
	})
	if !ok {
		return nil, ErrRepositoryReadUnsupported
	}
	return reader.GetByExperimentID(ctx, experimentID)
}

// ListByExperimentID is an alternate collection-oriented name.
func (s *Service) ListByExperimentID(ctx context.Context, experimentID string) ([]Batch, error) {
	return s.GetByExperimentID(ctx, experimentID)
}

// GetBatches retrieves batches by run when the configured repository supports
// the optional read side.
func (s *Service) GetBatches(ctx context.Context, runID string) ([]Batch, error) {
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	if s == nil || s.repository == nil {
		return nil, ErrMetricsRepositoryFailure
	}
	reader, ok := s.repository.(interface {
		GetBatches(context.Context, string) ([]Batch, error)
	})
	if !ok {
		return nil, ErrRepositoryReadUnsupported
	}
	return reader.GetBatches(ctx, runID)
}

// GetEvents retrieves flattened events by run when supported by the repository.
func (s *Service) GetEvents(ctx context.Context, runID string) ([]Event, error) {
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	if s == nil || s.repository == nil {
		return nil, ErrMetricsRepositoryFailure
	}
	reader, ok := s.repository.(interface {
		GetEvents(context.Context, string) ([]Event, error)
	})
	if !ok {
		return nil, ErrRepositoryReadUnsupported
	}
	return reader.GetEvents(ctx, runID)
}
