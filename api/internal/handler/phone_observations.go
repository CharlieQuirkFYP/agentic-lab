package handler

import (
	"errors"
	"net/http"
	"net/url"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phoneobservations"
	"github.com/gin-gonic/gin"
)

type PhoneObservationsHandler struct {
	service *phoneobservations.Service
	notify  func()
}

func NewPhoneObservationsHandler(service *phoneobservations.Service, notify func()) *PhoneObservationsHandler {
	return &PhoneObservationsHandler{service: service, notify: notify}
}
func (h *PhoneObservationsHandler) RegisterRoutes(g *gin.RouterGroup) {
	g.POST("/observations", h.Append)
	g.GET("/observations", h.List)
	g.GET("/legacy-observations", h.Legacy)
	g.GET("/overview", h.Overview)
	for _, path := range []string{"/observations", "/legacy-observations", "/overview"} {
		g.OPTIONS(path, func(c *gin.Context) { c.Status(http.StatusNoContent) })
	}
}
func (h *PhoneObservationsHandler) Append(c *gin.Context) {
	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, 4<<20)
	u, err := phoneobservations.DecodeUpload(c.Request.Body)
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			c.JSON(http.StatusRequestEntityTooLarge, gin.H{"error": "phone observation upload exceeds 4 MiB"})
		} else {
			c.JSON(http.StatusBadRequest, gin.H{"error": "invalid phone observation upload"})
		}
		return
	}
	result, err := h.service.Append(c.Request.Context(), u)
	if err != nil {
		observationError(c, err)
		return
	}
	if h.notify != nil {
		h.notify()
	}
	c.JSON(http.StatusAccepted, result)
}
func observationQuery(c *gin.Context, paginated, legacy bool) (phoneobservations.Query, error) {
	values, err := url.ParseQuery(c.Request.URL.RawQuery)
	if err != nil {
		return phoneobservations.Query{}, phoneobservations.ErrInvalidQuery
	}
	return phoneobservations.ParseQuery(values, paginated, legacy)
}
func (h *PhoneObservationsHandler) List(c *gin.Context) {
	q, err := observationQuery(c, true, false)
	if err != nil {
		observationError(c, err)
		return
	}
	p, err := h.service.List(c.Request.Context(), q)
	if err != nil {
		observationError(c, err)
		return
	}
	c.JSON(http.StatusOK, p)
}
func (h *PhoneObservationsHandler) Legacy(c *gin.Context) {
	q, err := observationQuery(c, true, true)
	if err != nil {
		observationError(c, err)
		return
	}
	p, err := h.service.ListLegacy(c.Request.Context(), q)
	if err != nil {
		observationError(c, err)
		return
	}
	c.JSON(http.StatusOK, p)
}
func (h *PhoneObservationsHandler) Overview(c *gin.Context) {
	q, err := observationQuery(c, false, false)
	if err != nil {
		observationError(c, err)
		return
	}
	p, err := h.service.Overview(c.Request.Context(), q)
	if err != nil {
		observationError(c, err)
		return
	}
	c.JSON(http.StatusOK, p)
}
func observationError(c *gin.Context, err error) {
	switch {
	case errors.Is(err, phoneobservations.ErrInvalidUpload):
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid phone observation upload"})
	case errors.Is(err, phoneobservations.ErrInvalidQuery):
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid phone observation query"})
	case errors.Is(err, phoneobservations.ErrConflict):
		c.JSON(http.StatusConflict, gin.H{"error": "phone observation identity already has different content"})
	default:
		c.JSON(http.StatusInternalServerError, gin.H{"error": "could not access phone observations"})
	}
}
