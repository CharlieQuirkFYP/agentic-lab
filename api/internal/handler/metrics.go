package handler

import (
	"errors"
	"net/http"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/gin-gonic/gin"
)

type MetricsHandler struct {
	service *metrics.Service
}

func NewMetricsHandler(service *metrics.Service) *MetricsHandler {
	return &MetricsHandler{service: service}
}

func (h *MetricsHandler) Append(c *gin.Context) {
	var batch metrics.Batch
	if err := c.ShouldBindJSON(&batch); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{
			"error": "invalid metrics batch",
		})
		return
	}

	experimentID := c.Param("id")
	if batch.ExperimentID == "" {
		batch.ExperimentID = experimentID
	}
	if err := batch.ValidateForExperiment(experimentID); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{
			"error": "invalid metrics batch",
		})
		return
	}

	if err := h.service.Append(c.Request.Context(), batch); err != nil {
		switch {
		case errors.Is(err, metrics.ErrInvalidBatch), errors.Is(err, metrics.ErrInvalidEvent):
			c.JSON(http.StatusBadRequest, gin.H{
				"error": "invalid metrics batch",
			})
		default:
			c.JSON(http.StatusInternalServerError, gin.H{
				"error": "failed to store metrics",
			})
		}
		return
	}

	c.JSON(http.StatusAccepted, gin.H{
		"status":      "accepted",
		"event_count": len(batch.Events),
	})
}

func (h *MetricsHandler) Get(c *gin.Context) {
	batches, err := h.service.GetByExperimentID(c.Request.Context(), c.Param("id"))
	if err != nil {
		if errors.Is(err, metrics.ErrMetricsRepositoryFailure) || errors.Is(err, metrics.ErrRepositoryReadUnsupported) {
			c.JSON(http.StatusInternalServerError, gin.H{
				"error": "failed to read metrics",
			})
			return
		}

		c.JSON(http.StatusInternalServerError, gin.H{
			"error": "failed to read metrics",
		})
		return
	}

	c.JSON(http.StatusOK, gin.H{
		"experiment_id": c.Param("id"),
		"batches":       batches,
	})
}
