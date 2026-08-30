package handler

import (
	"errors"
	"net/http"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/gin-gonic/gin"
)

type ExperimentHandler struct {
	benchmarkService *benchmark.Service
}

func NewExperimentHandler(benchmarkService *benchmark.Service) *ExperimentHandler {
	return &ExperimentHandler{
		benchmarkService: benchmarkService,
	}
}

func (h *ExperimentHandler) Create(c *gin.Context) {
	var req benchmark.CreateExperimentRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{
			"error": "invalid experiment request",
		})
		return
	}

	experiment, err := h.benchmarkService.CreateExperiment(c.Request.Context(), req)
	if err != nil {
		switch {
		case errors.Is(err, benchmark.ErrUseCaseRequired):
			c.JSON(http.StatusBadRequest, gin.H{
				"error": "use_case is required",
			})
		case errors.Is(err, benchmark.ErrUnsupportedUseCase):
			c.JSON(http.StatusBadRequest, gin.H{
				"error": "unsupported use_case",
			})
		default:
			c.JSON(http.StatusInternalServerError, gin.H{
				"error": "failed to create experiment",
			})
		}
		return
	}

	c.JSON(http.StatusAccepted, benchmark.CreateExperimentResponse{
		ID:     experiment.ID,
		Status: experiment.Status,
	})
}

func (h *ExperimentHandler) Get(c *gin.Context) {
	experiment, err := h.benchmarkService.GetExperiment(c.Request.Context(), c.Param("id"))
	if err != nil {
		if errors.Is(err, benchmark.ErrExperimentNotFound) {
			c.JSON(http.StatusNotFound, gin.H{
				"error": "experiment not found",
			})
			return
		}

		c.JSON(http.StatusInternalServerError, gin.H{
			"error": "failed to get experiment",
		})
		return
	}

	c.JSON(http.StatusOK, experiment)
}
