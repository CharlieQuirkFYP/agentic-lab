package handler

import (
	"errors"
	"net/http"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
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
	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, 4<<20)
	var batch metrics.Batch
	if err := c.ShouldBindJSON(&batch); err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			c.JSON(http.StatusRequestEntityTooLarge, gin.H{"error": "metrics batch exceeds 4 MiB"})
			return
		}
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
		case errors.Is(err, metrics.ErrBatchConflict):
			c.JSON(http.StatusConflict, gin.H{"error": "metrics batch identity already has different content"})
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
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	if err := validateQuery(values, "limit", "offset", "run_id", "name"); err != nil {
		dashboardError(c, err)
		return
	}
	query := metrics.Query{RunID: values.Get("run_id"), Name: values.Get("name")}
	if values.Has("limit") || values.Has("offset") {
		if !values.Has("limit") {
			dashboardError(c, benchmark.ErrInvalidQuery)
			return
		}
		limit, offset, err := pagination(values, 20)
		if err != nil {
			dashboardError(c, err)
			return
		}
		query.Limit = limit
		query.Offset = offset
	}
	page, err := h.service.List(c.Request.Context(), c.Param("id"), query)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{
			"error": "failed to read metrics",
		})
		return
	}

	c.JSON(http.StatusOK, page)
}
