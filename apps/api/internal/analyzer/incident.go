package analyzer

import (
	"context"

	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"
)

type IncidentAnalyzer interface {
	Analyze(ctx context.Context, req model.AnalyzeIncidentRequest) (model.IncidentReport, error)
}
