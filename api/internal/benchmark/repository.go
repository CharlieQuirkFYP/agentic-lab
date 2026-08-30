package benchmark

import (
	"context"
	"errors"
)

var ErrExperimentNotFound = errors.New("experiment not found")

type Repository interface {
	Create(ctx context.Context, experiment Experiment) error
	GetByID(ctx context.Context, id string) (Experiment, error)
	Update(ctx context.Context, experiment Experiment) error
}
