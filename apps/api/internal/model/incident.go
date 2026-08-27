package model

type AnalyzeIncidentRequest struct {
	Transcript string `json:"transcript" binding:"required"`
}

type IncidentReport struct {
	IncidentType      string `json:"incident_type"`
	Location          string `json:"location"`
	Severity          string `json:"severity"`
	Summary           string `json:"summary"`
	RecommendedAction string `json:"recommended_action"`
}
