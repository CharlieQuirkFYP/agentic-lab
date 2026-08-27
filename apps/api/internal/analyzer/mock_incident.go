package analyzer

import (
	"context"

	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"
)

var _ IncidentAnalyzer = (*MockIncidentAnalyzer)(nil)

type MockIncidentAnalyzer struct{}

func NewMockIncidentAnalyzer() *MockIncidentAnalyzer {
	return &MockIncidentAnalyzer{}
}

func (a *MockIncidentAnalyzer) Analyze(ctx context.Context, req model.AnalyzeIncidentRequest) (model.IncidentReport, error) {
	return model.IncidentReport{
		IncidentType:      "unknown",
		Location:          "unknown",
		Severity:          "unknown",
		Summary:           req.Transcript,
		RecommendedAction: "Pending AI analysis",
	}, nil
}
