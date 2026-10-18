package handler

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"mime"
	"net/http"
	"strconv"
	"strings"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/gin-gonic/gin"
)

const MaxVoiceUploadBytes = 32 << 20

type VoiceHandler struct {
	service *service.VoiceService
}

func NewVoiceHandler(service *service.VoiceService) *VoiceHandler {
	return &VoiceHandler{service: service}
}

func (h *VoiceHandler) RegisterRoutes(group *gin.RouterGroup) {
	for _, route := range []struct{ method, path, action string }{
		{"POST", "/conversations", "create"},
		{"GET", "/conversations/:conversation_id", "inspect"},
		{"POST", "/conversations/:conversation_id/finish", "finish"},
		{"GET", "/conversations/:conversation_id/metrics", "metrics"},
		{"GET", "/conversations/:conversation_id/runs/:run_id", "run"},
		{"POST", "/conversations/:conversation_id/turns", "start"},
		{"GET", "/conversations/:conversation_id/turns/:turn_id", "recover"},
		{"POST", "/conversations/:conversation_id/turns/:turn_id/submit", "submit"},
		{"POST", "/conversations/:conversation_id/turns/:turn_id/cancel", "cancel"},
	} {
		action := route.action
		group.Handle(route.method, route.path, func(c *gin.Context) { h.Conversation(c, action) })
		group.OPTIONS(route.path, func(c *gin.Context) { c.Status(http.StatusNoContent) })
	}
	group.POST("/turns", h.StartTurn)
	group.POST("/turns/:turn_id/submit", h.Submit)
	group.GET("/turns/:turn_id", h.TurnStatus)
	group.POST("/turns/:turn_id/cancel", h.Cancel)
	group.POST("/reset", h.Reset)
	group.GET("/inspect", h.Inspect)
	group.POST("/test/reply", h.TestReply)
	group.POST("/transcribe", h.Transcribe)
	for _, path := range []string{"/turns", "/turns/:turn_id/submit", "/turns/:turn_id", "/turns/:turn_id/cancel", "/reset", "/inspect", "/test/reply", "/transcribe"} {
		group.OPTIONS(path, func(c *gin.Context) { c.Status(http.StatusNoContent) })
	}
}

func (h *VoiceHandler) Conversation(c *gin.Context, action string) {
	op := pheme.ConversationOperation{Action: action, ConversationID: c.Param("conversation_id"), TurnID: c.Param("turn_id"), RunID: c.Param("run_id")}
	if action == "start" || action == "submit" {
		input, ok := voiceInput(c, action == "start", true)
		if !ok {
			return
		}
		op.Input = input
	}
	if action == "metrics" {
		cursor := uint64(0)
		if raw := c.Query("after"); raw != "" {
			var err error
			cursor, err = strconv.ParseUint(raw, 10, 64)
			if err != nil {
				voiceError(c, http.StatusBadRequest, "invalid_cursor", "invalid metrics cursor")
				return
			}
		}
		op.Input.MetricsCursor = &cursor
	}
	response, err := h.service.Conversation(c.Request.Context(), op)
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) StartTurn(c *gin.Context) {
	input, ok := voiceInput(c, true, true)
	if !ok {
		return
	}
	response, err := h.service.StartTurn(c.Request.Context(), input)
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) Submit(c *gin.Context) {
	input, ok := voiceInput(c, false, true)
	if !ok {
		return
	}
	response, err := h.service.Submit(c.Request.Context(), c.Param("turn_id"), input)
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) TurnStatus(c *gin.Context) {
	response, err := h.service.TurnStatus(c.Request.Context(), c.Param("turn_id"))
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) Cancel(c *gin.Context) {
	response, err := h.service.Cancel(c.Request.Context(), c.Param("turn_id"))
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) Reset(c *gin.Context) {
	response, err := h.service.Reset(c.Request.Context())
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) Inspect(c *gin.Context) {
	response, err := h.service.Inspect(c.Request.Context())
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) TestReply(c *gin.Context) {
	input, ok := voiceInput(c, false, true)
	if !ok {
		return
	}
	response, err := h.service.TestReply(c.Request.Context(), input)
	forwardVoice(c, response, err)
}

func (h *VoiceHandler) Transcribe(c *gin.Context) {
	input, ok := voiceInput(c, true, false)
	if !ok {
		return
	}
	response, err := h.service.Transcribe(c.Request.Context(), input)
	forwardVoice(c, response, err)
}

func voiceInput(c *gin.Context, allowAudio, allowText bool) (pheme.Input, bool) {
	contentType := c.GetHeader("Content-Type")
	mediaType, _, err := mime.ParseMediaType(contentType)
	isJSON := mediaType == "application/json"
	isWAV := mediaType == "audio/wav" || mediaType == "audio/x-wav"
	if err != nil || !(allowText && isJSON || allowAudio && isWAV) {
		voiceError(c, http.StatusUnsupportedMediaType, "unsupported_media_type", "unsupported voice Content-Type")
		return pheme.Input{}, false
	}
	key := c.GetHeader("Idempotency-Key")
	if len(key) > 256 || strings.ContainsFunc(key, func(r rune) bool { return r < 32 || r > 126 }) {
		voiceError(c, http.StatusBadRequest, "invalid_idempotency_key", "invalid Idempotency-Key")
		return pheme.Input{}, false
	}
	c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, MaxVoiceUploadBytes)
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		var sizeError *http.MaxBytesError
		if errors.As(err, &sizeError) {
			voiceError(c, http.StatusRequestEntityTooLarge, "body_too_large", "voice upload exceeds 32 MiB")
		} else if c.Request.Context().Err() == nil {
			voiceError(c, http.StatusBadRequest, "invalid_body", "could not read voice request")
		}
		return pheme.Input{}, false
	}
	if isJSON {
		var object map[string]json.RawMessage
		var text string
		if json.Unmarshal(body, &object) != nil || object["text"] == nil ||
			string(object["text"]) == "null" || json.Unmarshal(object["text"], &text) != nil {
			voiceError(c, http.StatusBadRequest, "invalid_body", "JSON body must contain a text string")
			return pheme.Input{}, false
		}
		// Pheme, not Go, validates blank/overlong text and approval/state rules.
	}
	source := c.GetHeader("X-Audio-Source")
	if len(source) > 512 || strings.ContainsFunc(source, func(r rune) bool { return r < 32 || r == 127 }) {
		voiceError(c, http.StatusBadRequest, "invalid_source", "invalid audio source")
		return pheme.Input{}, false
	}
	return pheme.Input{Body: body, ContentType: contentType, IdempotencyKey: key, AudioSource: source}, true
}

func forwardVoice(c *gin.Context, response *pheme.Response, err error) {
	if err == nil {
		// After streaming starts, transport failure closes the response; it must
		// not invent a Rust workflow event or append a JSON error to SSE bytes.
		err = response.Forward(c.Request.Context(), c.Writer)
		if err == nil || c.Writer.Written() {
			return
		}
	}
	if c.Request.Context().Err() != nil || errors.Is(err, context.Canceled) {
		return
	}
	switch {
	case errors.Is(err, pheme.ErrInvalidTurnID):
		voiceError(c, http.StatusBadRequest, "invalid_turn_id", "invalid turn ID")
	case errors.Is(err, pheme.ErrTimeout), errors.Is(err, context.DeadlineExceeded):
		voiceError(c, http.StatusGatewayTimeout, "upstream_timeout", "voice service timed out")
	default:
		voiceError(c, http.StatusBadGateway, "upstream_unavailable", "voice service unavailable")
	}
}

func voiceError(c *gin.Context, status int, code, message string) {
	c.AbortWithStatusJSON(status, gin.H{"error": gin.H{"code": code, "message": message}})
}
