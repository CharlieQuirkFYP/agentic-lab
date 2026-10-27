package benchmark

import (
	"context"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

type recordingAnalyzer struct {
	calls []struct {
		transcript, runID, experimentID string
	}
}

func (a *recordingAnalyzer) Analyze(_ context.Context, transcript, runID, experimentID string) (pheme.Analysis, error) {
	a.calls = append(a.calls, struct {
		transcript, runID, experimentID string
	}{transcript, runID, experimentID})
	return pheme.Analysis{IncidentType: "smoke"}, nil
}

func TestPhemeRunnerExecutesCasesWithCorrelation(t *testing.T) {
	analyzer := &recordingAnalyzer{}
	runner := NewPhemeRunner(analyzer)
	experiment := Experiment{ID: "exp-1", Cases: []Case{{ID: "case-1", Transcript: "Smoke near the gate"}, {ID: "case-2", Transcript: "Fire at the loading bay"}}}

	result, err := runner.Run(context.Background(), experiment)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Success || result.LatencyMS < 0 {
		t.Fatalf("result = %+v", result)
	}
	if len(analyzer.calls) != 2 {
		t.Fatalf("calls = %+v", analyzer.calls)
	}
	for _, call := range analyzer.calls {
		if call.runID != "run_exp-1" || call.experimentID != "exp-1" {
			t.Fatalf("correlation = %+v", call)
		}
	}
}

func TestPhemeRunnerRejectsEmptyCaseSet(t *testing.T) {
	runner := NewPhemeRunner(&recordingAnalyzer{})
	result, err := runner.Run(context.Background(), Experiment{ID: "exp-1"})
	if err != ErrNoCases || result.Success || result.ErrorMessage != ErrNoCases.Error() {
		t.Fatalf("result=%+v err=%v", result, err)
	}
}
