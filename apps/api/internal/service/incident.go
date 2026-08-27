package service

import (
	"context"

	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/analyzer"
	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"
)

type IncidentService struct {
	incidentAnalyzer analyzer.IncidentAnalyzer
}

func NewIncidentService(incidentAnalyzer analyzer.IncidentAnalyzer) *IncidentService {
	return &IncidentService{
		incidentAnalyzer: incidentAnalyzer,
	}
}

func (s *IncidentService) Analyze(ctx context.Context, req model.AnalyzeIncidentRequest) (model.IncidentReport, error) {
	return s.incidentAnalyzer.Analyze(ctx, req)
}
