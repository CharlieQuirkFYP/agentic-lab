package analyzer

import (
	"context"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/model"
)

func TestMockIncidentAnalyzerAnalyzeReturnsPlaceholderReport(t *testing.T) {
	analyzer := NewMockIncidentAnalyzer()
	req := model.AnalyzeIncidentRequest{
		Transcript: "A vehicle collided with a barrier near the west entrance.",
	}

	report, err := analyzer.Analyze(context.Background(), req)
	if err != nil {
		t.Fatalf("Analyze returned error: %v", err)
	}

	expected := model.IncidentReport{
		IncidentType:      "unknown",
		Location:          "unknown",
		Severity:          "unknown",
		Summary:           req.Transcript,
		RecommendedAction: "Pending AI analysis",
	}

	if report != expected {
		t.Fatalf("Analyze returned %+v, want %+v", report, expected)
	}
}
