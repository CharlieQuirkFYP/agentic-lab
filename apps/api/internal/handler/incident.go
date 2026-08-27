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

	report, err := h.incidentService.Analyze(c.Request.Context(), req)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{
			"error": "failed to analyze incident",
		})
		return
	}

	c.JSON(http.StatusOK, report)
}
