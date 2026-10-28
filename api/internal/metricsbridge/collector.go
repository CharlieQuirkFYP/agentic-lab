package metricsbridge

import (
	"context"
	"fmt"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

// Source is the small portion of Pheme needed by the benchmark integration.
// Keeping it as an interface makes the bridge easy to test without a server.
type Source interface {
	DrainMetrics(context.Context) ([]metrics.Batch, error)
}

// Collector drains Pheme's in-memory metric batches and persists them in the
// Go-owned experiment database used by the dashboard.
type Collector struct {
	source Source
	sink   *metrics.Service
}

func NewCollector(source Source, sink *metrics.Service) *Collector {
	return &Collector{source: source, sink: sink}
}

func (c *Collector) Collect(ctx context.Context, experiment benchmark.Experiment) error {
	if c == nil || c.source == nil || c.sink == nil {
		return fmt.Errorf("metrics collector is not configured")
	}
	batches, err := c.source.DrainMetrics(ctx)
	if err != nil {
		return err
	}
	for _, batch := range batches {
		if batch.ExperimentID == "" {
			batch.ExperimentID = experiment.ID
			for _, event := range batch.Events {
				if event.ExperimentID != "" {
					batch.ExperimentID = event.ExperimentID
					break
				}
			}
		}
		if err := c.sink.Append(ctx, batch); err != nil {
			return err
		}
	}
	return nil
}
