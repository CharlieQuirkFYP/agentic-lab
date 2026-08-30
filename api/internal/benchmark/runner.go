package benchmark

import "context"

type Runner interface {
	Run(ctx context.Context, experiment Experiment) (Result, error)
}
