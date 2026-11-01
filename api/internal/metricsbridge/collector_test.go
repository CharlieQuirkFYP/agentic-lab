package metricsbridge

import (
	"context"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

type fakeSource struct {
	batches []metrics.Batch
}

func (f fakeSource) DrainMetrics(context.Context) ([]metrics.Batch, error) {
	return f.batches, nil
}

func TestCollectorPersistsPhemeBatches(t *testing.T) {
	run := metrics.Batch{
		SchemaVersion:  1,
		RunID:          "run-1",
		SequenceNumber: 0,
		TimestampMS:    100,
		Events: []metrics.Event{{
			SchemaVersion: 1,
			RunID:         "run-1",
			Sequence:      0,
			TimestampMS:   100,
			Name:          "latency_ms",
			Value:         42.0,
			Unit:          "ms",
			Scope:         "run",
			Source:        "pheme",
		}},
	}
	service := metrics.NewService(metrics.NewMemoryRepository())
	collector := NewCollector(fakeSource{batches: []metrics.Batch{run}}, service)
	experiment := benchmark.Experiment{ID: "exp-1"}

	if err := collector.Collect(context.Background(), experiment); err != nil {
		t.Fatal(err)
	}
	stored, err := service.GetByExperimentID(context.Background(), experiment.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(stored) != 1 || stored[0].ExperimentID != experiment.ID {
		t.Fatalf("stored batches = %+v", stored)
	}
}
