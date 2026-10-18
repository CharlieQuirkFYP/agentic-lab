package handler

import (
	"errors"
	"net/http"
	"net/url"
	"strings"

	"github.com/gin-gonic/gin"
)

// NewVoiceCORS permits only explicitly configured browser origins. Requests
// without Origin (e.g. the local TUI) are allowed. This is not authentication;
// non-loopback deployment needs its own access controls and HTTPS.
func NewVoiceCORS(origins []string) (gin.HandlerFunc, error) {
	allowed := make(map[string]bool, len(origins))
	for _, origin := range origins {
		u, err := url.Parse(origin)
		if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || strings.Contains(u.Host, "*") ||
			u.User != nil || u.Path != "" || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.Opaque != "" {
			return nil, errors.New("voice CORS origins must be explicit http(s) origins without paths")
		}
		allowed[origin] = true
	}
	return func(c *gin.Context) {
		c.Writer.Header().Add("Vary", "Origin")
		origin := c.GetHeader("Origin")
		if origin == "" {
			c.Next()
			return
		}
		if !allowed[origin] {
			voiceError(c, http.StatusForbidden, "origin_not_allowed", "voice origin not allowed")
			return
		}
		c.Header("Access-Control-Allow-Origin", origin)
		if c.Request.Method == http.MethodOptions {
			c.Writer.Header().Add("Vary", "Access-Control-Request-Method")
			c.Writer.Header().Add("Vary", "Access-Control-Request-Headers")
			method := c.GetHeader("Access-Control-Request-Method")
			if method != http.MethodPost && method != http.MethodGet {
				voiceError(c, http.StatusForbidden, "cors_method_not_allowed", "voice CORS method not allowed")
				return
			}
			for _, name := range strings.Split(c.GetHeader("Access-Control-Request-Headers"), ",") {
				switch strings.ToLower(strings.TrimSpace(name)) {
				case "", "content-type", "idempotency-key", "accept":
				default:
					voiceError(c, http.StatusForbidden, "cors_header_not_allowed", "voice CORS header not allowed")
					return
				}
			}
			c.Header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
			c.Header("Access-Control-Allow-Headers", "Content-Type, Idempotency-Key, Accept")
			c.AbortWithStatus(http.StatusNoContent)
			return
		}
		c.Next()
	}, nil
}
