package service

import (
	"context"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"
)

type fakeIncidentAnalyzer struct {
	called bool
	ctx    context.Context
	req    model.AnalyzeIncidentRequest
	report model.IncidentReport
	err    error
}

func (f *fakeIncidentAnalyzer) Analyze(ctx context.Context, req model.AnalyzeIncidentRequest) (model.IncidentReport, error) {
	f.called = true
	f.ctx = ctx
	f.req = req

	return f.report, f.err
}

func TestIncidentServiceAnalyzeDelegatesToAnalyzer(t *testing.T) {
	ctx := context.Background()
	req := model.AnalyzeIncidentRequest{
		Transcript: "Smoke reported near the north stairwell.",
	}
	expected := model.IncidentReport{
		IncidentType:      "fire",
		Location:          "north stairwell",
		Severity:          "medium",
		Summary:           "Smoke reported near the north stairwell.",
		RecommendedAction: "Dispatch safety team",
	}

	analyzer := &fakeIncidentAnalyzer{
		report: expected,
	}
	service := NewIncidentService(analyzer)

	report, err := service.Analyze(ctx, req)
	if err != nil {
		t.Fatalf("Analyze returned error: %v", err)
	}
	if !analyzer.called {
		t.Fatal("Analyze did not call analyzer")
	}
	if analyzer.ctx != ctx {
		t.Fatal("Analyze did not pass request context to analyzer")
	}
	if analyzer.req != req {
		t.Fatalf("Analyze passed request %+v, want %+v", analyzer.req, req)
	}
	if report != expected {
		t.Fatalf("Analyze returned %+v, want %+v", report, expected)
	}
}
