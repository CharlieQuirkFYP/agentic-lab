package handler

import (
	"net/http"

	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/model"
	"github.com/CharlieQuirkFYP/agentic-lab/apps/api/internal/service"
	"github.com/gin-gonic/gin"
)

type IncidentHandler struct {
	incidentService *service.IncidentService
}

func NewIncidentHandler(incidentService *service.IncidentService) *IncidentHandler {
	return &IncidentHandler{
		incidentService: incidentService,
	}
}

func (h *IncidentHandler) Analyze(c *gin.Context) {
	var req model.AnalyzeIncidentRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{
			"error": "transcript is required",
		})
		return
	}

	report := h.incidentService.Analyze(req.Transcript)
	c.JSON(http.StatusOK, report)
}
