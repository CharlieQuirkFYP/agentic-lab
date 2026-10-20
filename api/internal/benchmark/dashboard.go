package benchmark

import (
	"context"
	"encoding/json"
	"math"
	"sort"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

// MetricSummary groups only identical name/unit/scope/source combinations.
// Means describe reported observations, not per-task quality or energy scores.
type MetricSummary struct {
	Name               string   `json:"name"`
	Unit               string   `json:"unit"`
	Scope              string   `json:"scope"`
	Source             string   `json:"source"`
	ObservationCount   int      `json:"observation_count"`
	NumericCount       int      `json:"numeric_count"`
	UnavailableCount   int      `json:"unavailable_count"`
	UnavailableReasons []string `json:"unavailable_reasons"`
	Min                *float64 `json:"min"`
	Max                *float64 `json:"max"`
	Mean               *float64 `json:"mean"`
}

type RunSummary struct {
	RunID            string          `json:"run_id"`
	BatchCount       int             `json:"batch_count"`
	EventCount       int             `json:"event_count"`
	FirstTimestampMS int64           `json:"first_timestamp_ms"`
	LastTimestampMS  int64           `json:"last_timestamp_ms"`
	Metrics          []MetricSummary `json:"metrics"`
}

type RunPage struct {
	ExperimentID string       `json:"experiment_id"`
	Runs         []RunSummary `json:"runs"`
	Total        int          `json:"total"`
	Limit        int          `json:"limit"`
	Offset       int          `json:"offset"`
}

type Summary struct {
	ExperimentID string           `json:"experiment_id"`
	Status       ExperimentStatus `json:"status"`
	ResultSource string           `json:"result_source"`
	Result       *Result          `json:"result"`
	RunCount     int              `json:"run_count"`
	Metrics      []MetricSummary  `json:"metrics"`
}

type Export struct {
	SchemaVersion int             `json:"schema_version"`
	Experiment    Experiment      `json:"experiment"`
	Summary       Summary         `json:"summary"`
	Runs          []RunSummary    `json:"runs"`
	Batches       []metrics.Batch `json:"batches"`
}

type DashboardService struct {
	experiments *Service
	metrics     *metrics.Service
}

func NewDashboardService(experiments *Service, measurements *metrics.Service) *DashboardService {
	return &DashboardService{experiments: experiments, metrics: measurements}
}

func (s *DashboardService) Export(ctx context.Context, id string) (Export, error) {
	e, err := s.experiments.GetExperiment(ctx, id)
	if err != nil {
		return Export{}, err
	}
	batches, err := s.metrics.GetByExperimentID(ctx, id)
	if err != nil {
		return Export{}, err
	}
	sort.Slice(batches, func(i, j int) bool {
		if batches[i].RunID == batches[j].RunID {
			return batches[i].SequenceNumber < batches[j].SequenceNumber
		}
		return batches[i].RunID < batches[j].RunID
	})
	if batches == nil {
		batches = []metrics.Batch{}
	}
	runs := summarizeRuns(batches)
	source := e.ResultSource
	if source == "" {
		source = "unavailable"
	}
	return Export{SchemaVersion: 1, Experiment: e, Summary: Summary{ExperimentID: id, Status: e.Status, ResultSource: source, Result: e.Result, RunCount: len(runs), Metrics: summarizeMetrics(batches)}, Runs: runs, Batches: batches}, nil
}

func (s *DashboardService) Summary(ctx context.Context, id string) (Summary, error) {
	data, err := s.Export(ctx, id)
	return data.Summary, err
}

func (s *DashboardService) Runs(ctx context.Context, id string, limit, offset int) (RunPage, error) {
	if limit < 1 || limit > 100 || offset < 0 {
		return RunPage{}, ErrInvalidQuery
	}
	data, err := s.Export(ctx, id)
	if err != nil {
		return RunPage{}, err
	}
	page := RunPage{ExperimentID: id, Runs: []RunSummary{}, Total: len(data.Runs), Limit: limit, Offset: offset}
	if offset < len(data.Runs) {
		page.Runs = data.Runs[offset : offset+min(limit, len(data.Runs)-offset)]
	}
	return page, nil
}

func summarizeRuns(batches []metrics.Batch) []RunSummary {
	groups := map[string][]metrics.Batch{}
	for _, b := range batches {
		groups[b.RunID] = append(groups[b.RunID], b)
	}
	runs := []RunSummary{}
	for id, group := range groups {
		r := RunSummary{RunID: id, BatchCount: len(group), FirstTimestampMS: math.MaxInt64, Metrics: summarizeMetrics(group)}
		for _, b := range group {
			r.FirstTimestampMS = min(r.FirstTimestampMS, b.TimestampMS)
			r.LastTimestampMS = max(r.LastTimestampMS, b.TimestampMS)
			r.EventCount += len(b.Events)
			for _, e := range b.Events {
				r.FirstTimestampMS = min(r.FirstTimestampMS, e.TimestampMS)
				r.LastTimestampMS = max(r.LastTimestampMS, e.TimestampMS)
			}
		}
		runs = append(runs, r)
	}
	sort.Slice(runs, func(i, j int) bool { return runs[i].RunID < runs[j].RunID })
	return runs
}

func summarizeMetrics(batches []metrics.Batch) []MetricSummary {
	type key struct{ name, unit, scope, source string }
	groups := map[key]*MetricSummary{}
	for _, b := range batches {
		for _, e := range b.Events {
			k := key{e.Name, e.Unit, e.Scope, e.Source}
			g := groups[k]
			if g == nil {
				g = &MetricSummary{Name: e.Name, Unit: e.Unit, Scope: e.Scope, Source: e.Source, UnavailableReasons: []string{}}
				groups[k] = g
			}
			g.ObservationCount++
			if e.Value == nil {
				g.UnavailableCount++
				found := false
				for _, reason := range g.UnavailableReasons {
					if reason == e.UnavailableReason {
						found = true
					}
				}
				if !found {
					g.UnavailableReasons = append(g.UnavailableReasons, e.UnavailableReason)
				}
				continue
			}
			// JSON conversion accepts the numeric Go scalar types supported by Event.
			encoded, _ := json.Marshal(e.Value)
			var value float64
			if string(encoded) == "null" || json.Unmarshal(encoded, &value) != nil || math.IsNaN(value) || math.IsInf(value, 0) {
				continue
			}
			g.NumericCount++
			if g.Mean == nil {
				low, high, mean := value, value, value
				g.Min = &low
				g.Max = &high
				g.Mean = &mean
			} else {
				*g.Min = math.Min(*g.Min, value)
				*g.Max = math.Max(*g.Max, value)
				// Weighted form avoids overflow in value - mean for opposite signs.
				count := float64(g.NumericCount)
				*g.Mean = *g.Mean*((count-1)/count) + value/count
			}
		}
	}
	result := []MetricSummary{}
	for _, g := range groups {
		sort.Strings(g.UnavailableReasons)
		result = append(result, *g)
	}
	sort.Slice(result, func(i, j int) bool {
		a, b := result[i], result[j]
		if a.Name != b.Name {
			return a.Name < b.Name
		}
		if a.Unit != b.Unit {
			return a.Unit < b.Unit
		}
		if a.Scope != b.Scope {
			return a.Scope < b.Scope
		}
		return a.Source < b.Source
	})
	return result
}
