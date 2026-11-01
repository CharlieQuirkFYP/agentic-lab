package benchmark

import (
	"context"
	"errors"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

type Runner interface {
	Run(ctx context.Context, experiment Experiment) (Result, error)
}

var ErrNoCases = errors.New("benchmark has no cases")

type Analyzer interface {
	Analyze(context.Context, string, string, string) (pheme.Analysis, error)
}

type PhemeRunner struct {
	analyzer Analyzer
}

func NewPhemeRunner(analyzer Analyzer) *PhemeRunner {
	return &PhemeRunner{analyzer: analyzer}
}

func (r *PhemeRunner) ResultSource() string { return "measured" }

func (r *PhemeRunner) Run(ctx context.Context, experiment Experiment) (Result, error) {
	if len(experiment.Cases) == 0 {
		return Result{Success: false, ErrorMessage: ErrNoCases.Error()}, ErrNoCases
	}
	if r == nil || r.analyzer == nil {
		err := errors.New("Pheme analyzer is not configured")
		return Result{Success: false, ErrorMessage: err.Error()}, err
	}

	started := time.Now()
	runID := "run_" + experiment.ID
	var firstErr error
	for _, testCase := range experiment.Cases {
		if _, err := r.analyzer.Analyze(ctx, testCase.Transcript, runID, experiment.ID); err != nil && firstErr == nil {
			firstErr = err
		}
	}

	result := Result{
		LatencyMS: int(time.Since(started) / time.Millisecond),
		Success:   firstErr == nil,
	}
	if firstErr != nil {
		result.ErrorMessage = firstErr.Error()
	}
	return result, firstErr
}
