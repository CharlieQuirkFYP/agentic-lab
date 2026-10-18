package handler

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
)

func TestVoiceCORSRejectsNonExplicitOrigins(t *testing.T) {
	for _, origin := range []string{"*", "http://*", "http://*.example.com", "null", "", "ftp://localhost", "http://localhost/", "http://localhost/path", "http://user:pass@localhost", "http://localhost?x=y", "http://localhost?", "http://localhost/#x"} {
		if _, err := NewVoiceCORS([]string{origin}); err == nil {
			t.Errorf("NewVoiceCORS accepted %q", origin)
		}
	}
}

func TestVoiceCORSGuard(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct {
		name           string
		origins        []string
		origin         string
		method         string
		requestMethod  string
		requestHeaders string
		want           int
	}{
		{name: "local nonbrowser", method: http.MethodPost, want: http.StatusNoContent},
		{name: "browser denied by default", origin: "http://localhost:5173", method: http.MethodPost, want: http.StatusForbidden},
		{name: "explicit browser", origins: []string{"http://localhost:5173"}, origin: "http://localhost:5173", method: http.MethodPost, want: http.StatusNoContent},
		{name: "different hostname denied", origins: []string{"http://localhost:5173"}, origin: "http://127.0.0.1:5173", method: http.MethodPost, want: http.StatusForbidden},
		{name: "untrusted origin", origins: []string{"http://localhost:5173"}, origin: "https://untrusted.example", method: http.MethodPost, want: http.StatusForbidden},
		{name: "preflight", origins: []string{"http://localhost:5173"}, origin: "http://localhost:5173", method: http.MethodOptions, requestMethod: "POST", requestHeaders: "Content-Type, Idempotency-Key", want: http.StatusNoContent},
		{name: "unsupported method", origins: []string{"http://localhost:5173"}, origin: "http://localhost:5173", method: http.MethodOptions, requestMethod: "DELETE", want: http.StatusForbidden},
		{name: "unsupported header", origins: []string{"http://localhost:5173"}, origin: "http://localhost:5173", method: http.MethodOptions, requestMethod: "POST", requestHeaders: "X-System-Prompt", want: http.StatusForbidden},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cors, err := NewVoiceCORS(tc.origins)
			if err != nil {
				t.Fatal(err)
			}
			called := false
			router := gin.New()
			group := router.Group("/api/v1/voice", cors)
			group.Match([]string{http.MethodPost, http.MethodOptions}, "/reset", func(c *gin.Context) {
				called = true
				c.Status(http.StatusNoContent)
			})
			req := httptest.NewRequest(tc.method, "/api/v1/voice/reset", nil)
			req.Header.Set("Origin", tc.origin)
			req.Header.Set("Access-Control-Request-Method", tc.requestMethod)
			req.Header.Set("Access-Control-Request-Headers", tc.requestHeaders)
			resp := httptest.NewRecorder()
			router.ServeHTTP(resp, req)
			if resp.Code != tc.want {
				t.Fatalf("status=%d, want %d; %s", resp.Code, tc.want, resp.Body.String())
			}
			if tc.want == http.StatusForbidden && called {
				t.Fatal("denied origin reached handler")
			}
			if tc.method == http.MethodOptions && called {
				t.Fatal("preflight reached mutation handler")
			}
			if !strings.Contains(strings.Join(resp.Header().Values("Vary"), ","), "Origin") {
				t.Fatal("response must vary by Origin")
			}
			if tc.want == http.StatusNoContent && tc.origin != "" && resp.Header().Get("Access-Control-Allow-Origin") != tc.origin {
				t.Fatal("missing exact allowed origin")
			}
			if resp.Header().Get("Access-Control-Allow-Credentials") != "" {
				t.Fatal("must not enable credentialed CORS")
			}
		})
	}
}
