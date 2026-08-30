package benchmark

import (
	"context"
	"time"
)

type MockRunner struct {
	workDuration time.Duration
}

func NewMockRunner() *MockRunner {
	return &MockRunner{
		workDuration: 50 * time.Millisecond,
	}
}

func (r *MockRunner) Run(ctx context.Context, experiment Experiment) (Result, error) {
	timer := time.NewTimer(r.workDuration)
	defer timer.Stop()

	select {
	case <-ctx.Done():
		return Result{
			Success:      false,
			ErrorMessage: ctx.Err().Error(),
		}, ctx.Err()
	case <-timer.C:
		return Result{
			LatencyMS:       123,
			PeakMemoryMB:    256,
			TokensPerSecond: 18.5,
			Success:         true,
		}, nil
	}
}
