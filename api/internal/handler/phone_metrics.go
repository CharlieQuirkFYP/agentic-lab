package handler

import (
	"errors"
	"net/http"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
	"github.com/gin-gonic/gin"
)

type PhoneMetricsHandler struct {
	store  *phonemetrics.Store
	notify func()
}

func NewPhoneMetricsHandler(store *phonemetrics.Store, notify func()) *PhoneMetricsHandler {
	return &PhoneMetricsHandler{store: store, notify: notify}
}

func (h *PhoneMetricsHandler) RegisterRoutes(group *gin.RouterGroup) {
	group.POST("/metrics", h.Append)
	group.GET("/metrics", h.Get)
	group.OPTIONS("/metrics", func(c *gin.Context) { c.Status(http.StatusNoContent) })
}

func (h *PhoneMetricsHandler) Append(c *gin.Context) {
	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, 4<<20)
	var upload phonemetrics.Upload
	if err := c.ShouldBindJSON(&upload); err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			c.JSON(http.StatusRequestEntityTooLarge, gin.H{"error": "phone metrics upload exceeds 4 MiB"})
		} else {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid phone metrics upload"})
		}
		return
	}
	id, err := h.store.Append(c.Request.Context(), upload)
	if err != nil {
		switch {
		case errors.Is(err, phonemetrics.ErrInvalidUpload):
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid phone metrics upload"})
		case errors.Is(err, metrics.ErrBatchConflict):
			c.JSON(http.StatusConflict, gin.H{"error": "phone metrics identity already has different content"})
		default:
			c.JSON(http.StatusInternalServerError, gin.H{"error": "could not store phone metrics"})
		}
		return
	}
	if h.notify != nil {
		h.notify()
	}
	c.JSON(http.StatusAccepted, gin.H{"status": "accepted", "experiment_id": id, "event_count": len(upload.Batch.Events)})
}

func (h *PhoneMetricsHandler) Get(c *gin.Context) {
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	if err := validateQuery(values, "limit", "offset"); err != nil {
		dashboardError(c, err)
		return
	}
	limit, offset, err := pagination(values, 100)
	if err != nil {
		dashboardError(c, err)
		return
	}
	page, err := h.store.List(c.Request.Context(), limit, offset)
	if err != nil {
		dashboardError(c, err)
		return
	}
	c.JSON(http.StatusOK, page)
}
