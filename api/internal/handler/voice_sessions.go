package handler

import (
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/voicehistory"
	"github.com/gin-gonic/gin"
	"net/http"
)

type VoiceSessionsHandler struct{ store *voicehistory.Store }

func NewVoiceSessionsHandler(store *voicehistory.Store) *VoiceSessionsHandler {
	return &VoiceSessionsHandler{store: store}
}

func (h *VoiceSessionsHandler) List(c *gin.Context) {
	sessions, err := h.store.List(c.Request.Context())
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "could not read voice sessions"})
		return
	}
	c.JSON(http.StatusOK, gin.H{"sessions": sessions, "total": len(sessions)})
}
