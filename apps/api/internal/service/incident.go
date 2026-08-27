package service

import "github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"

type IncidentService struct{}

func NewIncidentService() *IncidentService {
	return &IncidentService{}
}

func (s *IncidentService) Analyze(transcript string) model.IncidentReport {
	return model.IncidentReport{
		IncidentType:      "unknown",
		Location:          "unknown",
		Severity:          "unknown",
		Summary:           transcript,
		RecommendedAction: "Pending AI analysis",
	}
}
