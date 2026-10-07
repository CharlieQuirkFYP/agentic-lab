package handler

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/gin-gonic/gin"
)

const voiceSnapshot = `{"history":[{"role":"user","content":"web question"},{"role":"assistant","content":"web answer"}],"current_turn":{"turn_id":"web-1","status":"awaiting_review","transcript":"café","approved_text":null,"reply":"","error":null,"timings":{"stt_ms":12}},"stt":{"name":"fake-stt","ready":true},"reply":{"name":"fake-reply","ready":true},"role":{"name":"incident","sha256":"abc"},"busy":false,"future_field":42}`

func TestVoiceRoutesForwardBodiesHeadersAndStatus(t *testing.T) {
	for _, tc := range []struct {
		name         string
		method       string
		publicPath   string
		upstreamPath string
		contentType  string
		body         string
		status       int
		stream       bool
	}{
		{"text turn", "POST", "/turns", "/v1/voice/turns", "application/json; charset=utf-8", " {\"text\": \"café\\nquestion\", \"future\": true} \n", http.StatusOK, true},
		{"WAV turn", "POST", "/turns", "/v1/voice/turns", "audio/wav", "RIFF\x00\x01WAVEfixture", http.StatusOK, true},
		{"known retry JSON", "POST", "/turns", "/v1/voice/turns", "application/json", `{"text":"known question"}`, http.StatusOK, false},
		{"submit", "POST", "/turns/web-1/submit", "/v1/voice/turns/web-1/submit", "application/json", `{"text":"exact edited question"}`, http.StatusAccepted, false},
		{"status", "GET", "/turns/web-1", "/v1/voice/turns/web-1", "", "", http.StatusOK, false},
		{"cancel", "POST", "/turns/web-1/cancel", "/v1/voice/turns/web-1/cancel", "", "", http.StatusOK, false},
		{"reset", "POST", "/reset", "/v1/voice/reset", "", "", http.StatusNoContent, false},
		{"inspect", "GET", "/inspect", "/v1/voice/inspect", "", "", http.StatusOK, false},
		{"isolated reply", "POST", "/test/reply", "/v1/voice/test/reply", "application/json", `{"text":"test only"}`, http.StatusOK, true},
		{"transcribe", "POST", "/transcribe", "/v1/transcribe", "audio/wav; charset=binary", "RIFF\x00WAVEtest-only", http.StatusOK, false},
		{"WAV alias", "POST", "/transcribe", "/v1/transcribe", "audio/x-wav", "RIFF\x00WAVEtest-only", http.StatusOK, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			wantBody := voiceSnapshot
			if tc.stream {
				wantBody = "event: turn.created\ndata: {\"turn_id\":\"web-1\"}\n\n: heartbeat\n\n"
			} else if tc.status == http.StatusNoContent {
				wantBody = ""
			}
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				body, err := io.ReadAll(r.Body)
				if err != nil || string(body) != tc.body || r.Method != tc.method || r.URL.Path != tc.upstreamPath || r.URL.RawQuery != "" {
					t.Errorf("upstream request: %s %s body=%q err=%v", r.Method, r.URL, body, err)
				}
				if r.Header.Get("Content-Type") != tc.contentType {
					t.Errorf("Content-Type = %q, want %q", r.Header.Get("Content-Type"), tc.contentType)
				}
				if tc.body != "" && r.Header.Get("Idempotency-Key") != "request-1" {
					t.Error("Idempotency-Key was not forwarded")
				}
				for _, name := range []string{"Authorization", "Cookie", "Origin", "X-System-Prompt"} {
					if r.Header.Get(name) != "" {
						t.Errorf("untrusted header %s forwarded", name)
					}
				}
				if tc.stream {
					if r.Header.Get("Accept") != "text/event-stream" {
						t.Error("stream request must accept SSE")
					}
					w.Header().Set("Content-Type", "text/event-stream; charset=utf-8")
				} else {
					w.Header().Set("Content-Type", "application/json")
				}
				w.Header().Set("Set-Cookie", "upstream-secret=1")
				w.Header().Set("X-Private-Debug", "internal detail")
				w.WriteHeader(tc.status)
				_, _ = io.WriteString(w, wantBody)
			}))
			t.Cleanup(upstream.Close)
			router := setupVoiceRouter(t, upstream.URL)
			req := httptest.NewRequest(tc.method, "/api/v1/voice"+tc.publicPath+"?ignored=1", strings.NewReader(tc.body))
			req.Header.Set("Content-Type", tc.contentType)
			req.Header.Set("Idempotency-Key", "request-1")
			req.Header.Set("Authorization", "secret")
			req.Header.Set("Cookie", "private=1")
			req.Header.Set("X-System-Prompt", "do not forward")
			resp := httptest.NewRecorder()
			router.ServeHTTP(resp, req)
			if resp.Code != tc.status || resp.Body.String() != wantBody || calls.Load() != 1 {
				t.Fatalf("response: status=%d body=%q upstream calls=%d", resp.Code, resp.Body.String(), calls.Load())
			}
			if resp.Header().Get("Set-Cookie") != "" || resp.Header().Get("X-Private-Debug") != "" {
				t.Fatal("private upstream headers leaked")
			}
			if tc.stream && (!resp.Flushed || resp.Header().Get("X-Accel-Buffering") != "no") {
				t.Fatal("SSE must flush and disable proxy buffering")
			}
		})
	}
}

func TestVoiceUpstreamErrorsRemainAuthoritative(t *testing.T) {
	for _, status := range []int{http.StatusBadRequest, http.StatusNotFound, http.StatusConflict, http.StatusRequestEntityTooLarge, http.StatusTooManyRequests, http.StatusServiceUnavailable} {
		t.Run(http.StatusText(status), func(t *testing.T) {
			body := " {\"error\": {\"code\":\"rust_decision\",\"message\":\"not accepted\"},\"future\":123}\n"
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				w.Header().Set("Retry-After", "3")
				w.WriteHeader(status)
				_, _ = io.WriteString(w, body)
			}))
			t.Cleanup(upstream.Close)
			router := setupVoiceRouter(t, upstream.URL)
			resp := voiceRequest(router, "POST", "/turns", "application/json", `{"text":"question"}`)
			if resp.Code != status || resp.Body.String() != body || resp.Header().Get("Retry-After") != "3" {
				t.Fatalf("Pheme error changed: status=%d body=%q headers=%v", resp.Code, resp.Body.String(), resp.Header())
			}
		})
	}
}

func TestVoiceRejectsInvalidHTTPInputsWithoutCallingPheme(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls.Add(1)
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer upstream.Close()
	router := setupVoiceRouter(t, upstream.URL)
	for _, tc := range []struct {
		name        string
		path        string
		contentType string
		body        string
		want        int
	}{
		{"WebM", "/turns", "audio/webm", "data", http.StatusUnsupportedMediaType},
		{"missing media type", "/turns", "", `{"text":"x"}`, http.StatusUnsupportedMediaType},
		{"invalid media type", "/turns", "application/json; invalid", `{"text":"x"}`, http.StatusUnsupportedMediaType},
		{"malformed JSON", "/turns", "application/json", `{"text":`, http.StatusBadRequest},
		{"missing text", "/turns", "application/json", `{}`, http.StatusBadRequest},
		{"number text", "/turns", "application/json", `{"text":42}`, http.StatusBadRequest},
		{"null text", "/turns", "application/json", `{"text":null}`, http.StatusBadRequest},
		{"array", "/turns", "application/json", `[]`, http.StatusBadRequest},
		{"null object", "/turns", "application/json", `null`, http.StatusBadRequest},
		{"multiple JSON bodies", "/turns", "application/json", `{"text":"x"} {"text":"y"}`, http.StatusBadRequest},
		{"submit audio", "/turns/web-1/submit", "audio/wav", "RIFF", http.StatusUnsupportedMediaType},
		{"test reply audio", "/test/reply", "audio/wav", "RIFF", http.StatusUnsupportedMediaType},
		{"transcribe JSON", "/transcribe", "application/json", `{"text":"x"}`, http.StatusUnsupportedMediaType},
	} {
		t.Run(tc.name, func(t *testing.T) {
			resp := voiceRequest(router, "POST", tc.path, tc.contentType, tc.body)
			if resp.Code != tc.want {
				t.Fatalf("status=%d, want %d; %s", resp.Code, tc.want, resp.Body.String())
			}
		})
	}
	for _, key := range []string{strings.Repeat("x", 257), "key\nsecret"} {
		req := httptest.NewRequest("POST", "/api/v1/voice/turns", strings.NewReader(`{"text":"x"}`))
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Idempotency-Key", key)
		resp := httptest.NewRecorder()
		router.ServeHTTP(resp, req)
		if resp.Code != http.StatusBadRequest {
			t.Fatalf("invalid key status=%d", resp.Code)
		}
	}
	for _, length := range []int64{-1, MaxVoiceUploadBytes + 1} {
		req := httptest.NewRequest("POST", "/api/v1/voice/turns", io.LimitReader(repeatedByteReader{}, MaxVoiceUploadBytes+1))
		req.ContentLength = length
		req.Header.Set("Content-Type", "audio/wav")
		resp := httptest.NewRecorder()
		router.ServeHTTP(resp, req)
		if resp.Code != http.StatusRequestEntityTooLarge {
			t.Fatalf("oversized upload status=%d, want 413", resp.Code)
		}
	}
	if calls.Load() != 0 {
		t.Fatalf("invalid HTTP requests reached Pheme %d times", calls.Load())
	}
}

func TestVoiceDoesNotValidateWorkflowText(t *testing.T) {
	const body = `{"text":"  "}`
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got, _ := io.ReadAll(r.Body)
		if string(got) != body {
			t.Errorf("input changed: %q", got)
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusUnprocessableEntity)
		_, _ = io.WriteString(w, `{"error":{"code":"empty_text","message":"Pheme validates this"}}`)
	}))
	defer upstream.Close()
	resp := voiceRequest(setupVoiceRouter(t, upstream.URL), "POST", "/turns/web-1/submit", "application/json", body)
	if resp.Code != http.StatusUnprocessableEntity {
		t.Fatalf("Go applied workflow validation: status=%d", resp.Code)
	}
}

func TestVoiceStreamFlushesBeforeFinishAndSubmitReachesOriginalRequest(t *testing.T) {
	approved := make(chan struct{})
	finish := make(chan struct{})
	finished := make(chan struct{})
	var finishOnce sync.Once
	release := func() { finishOnce.Do(func() { close(finish) }) }
	t.Cleanup(release)
	// Deliberately split a Unicode codepoint and SSE frame at the review wait.
	fullTranscript := "event: transcript.ready\ndata: {\"turn_id\":\"web-1\",\"text\":\"café\"}\n\n"
	split := strings.Index(fullTranscript, "é") + 1
	prefix := "event: turn.created\ndata: {\"turn_id\":\"web-1\"}\n\n: heartbeat\n\n" + fullTranscript[:split]
	suffix := fullTranscript[split:] + "event: question.approved\ndata: {\"turn_id\":\"web-1\",\"text\":\"corrected\"}\n\nevent: reply.started\ndata: {\"turn_id\":\"web-1\"}\n\nevent: reply.delta\ndata: {\"turn_id\":\"web-1\",\"text\":\"answer\"}\n\n"
	terminal := "event: reply.completed\ndata: {\"turn_id\":\"web-1\",\"text\":\"answer\"}\n\n"
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/voice/turns":
			defer close(finished)
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, prefix)
			w.(http.Flusher).Flush()
			select {
			case <-approved:
			case <-r.Context().Done():
				return
			}
			_, _ = io.WriteString(w, suffix)
			w.(http.Flusher).Flush()
			select {
			case <-finish:
				_, _ = io.WriteString(w, terminal)
			case <-r.Context().Done():
			}
		case "/v1/voice/turns/web-1/submit":
			body, _ := io.ReadAll(r.Body)
			if string(body) != `{"text":"corrected"}` {
				t.Errorf("approved body=%q", body)
			}
			close(approved)
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusAccepted)
			_, _ = io.WriteString(w, `{"turn_id":"web-1","status":"generating"}`)
		default:
			t.Errorf("unexpected path: %s", r.URL.Path)
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	t.Cleanup(upstream.Close)
	public := httptest.NewServer(setupVoiceRouter(t, upstream.URL))
	t.Cleanup(public.Close)
	client := &http.Client{Timeout: 5 * time.Second}
	stream, err := client.Post(public.URL+"/api/v1/voice/turns", "application/json", strings.NewReader(`{"text":"original"}`))
	if err != nil {
		t.Fatal(err)
	}
	defer stream.Body.Close()
	if stream.StatusCode != http.StatusOK {
		t.Fatalf("stream status=%d", stream.StatusCode)
	}
	readExact(t, stream.Body, prefix)
	select {
	case <-finished:
		t.Fatal("upstream finished before first bytes were flushed")
	default:
	}
	submit, err := client.Post(public.URL+"/api/v1/voice/turns/web-1/submit", "application/json", strings.NewReader(`{"text":"corrected"}`))
	if err != nil {
		t.Fatal(err)
	}
	submitBody, _ := io.ReadAll(submit.Body)
	_ = submit.Body.Close()
	if submit.StatusCode != http.StatusAccepted || strings.Contains(string(submitBody), "reply.delta") {
		t.Fatalf("submit did not acknowledge separately: status=%d body=%q", submit.StatusCode, submitBody)
	}
	readExact(t, stream.Body, suffix)
	release()
	rest, err := io.ReadAll(stream.Body)
	if err != nil || string(rest) != terminal {
		t.Fatalf("terminal=%q error=%v", rest, err)
	}
}

func TestVoiceConcurrentSubmissionsAreNotSerializedInGo(t *testing.T) {
	entered := make(chan struct{}, 2)
	release := make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	defer unblock()
	var accepted atomic.Bool
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/voice/turns/web-1/submit" {
			t.Errorf("unexpected path: %s", r.URL.Path)
		}
		body, _ := io.ReadAll(r.Body)
		if string(body) != `{"text":"first"}` && string(body) != `{"text":"second"}` {
			t.Errorf("unexpected submission body: %q", body)
		}
		entered <- struct{}{}
		select {
		case <-release:
		case <-r.Context().Done():
			return
		}
		w.Header().Set("Content-Type", "application/json")
		if accepted.CompareAndSwap(false, true) {
			w.WriteHeader(http.StatusAccepted)
			_, _ = io.WriteString(w, `{"status":"generating"}`)
		} else {
			w.WriteHeader(http.StatusConflict)
			_, _ = io.WriteString(w, `{"error":{"code":"already_approved","message":"frozen question"}}`)
		}
	}))
	t.Cleanup(upstream.Close)
	public := httptest.NewServer(setupVoiceRouter(t, upstream.URL))
	t.Cleanup(public.Close)
	client := &http.Client{Timeout: 5 * time.Second}
	results := make(chan int, 2)
	for _, body := range []string{`{"text":"first"}`, `{"text":"second"}`} {
		go func() {
			resp, err := client.Post(public.URL+"/api/v1/voice/turns/web-1/submit", "application/json", strings.NewReader(body))
			if err != nil {
				t.Errorf("submit failed: %v", err)
				results <- 0
				return
			}
			_, _ = io.Copy(io.Discard, resp.Body)
			_ = resp.Body.Close()
			results <- resp.StatusCode
		}()
	}
	for range 2 {
		select {
		case <-entered:
		case <-time.After(2 * time.Second):
			t.Fatal("concurrent submission did not reach Pheme while the other was pending")
		}
	}
	unblock()
	statuses := map[int]int{<-results: 1}
	statuses[<-results]++
	if statuses[http.StatusAccepted] != 1 || statuses[http.StatusConflict] != 1 {
		t.Fatalf("Pheme statuses not preserved: %v", statuses)
	}
}

func TestVoiceTestReplyAndTranscribeAreIsolatedFromWebStream(t *testing.T) {
	release := make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	defer unblock()
	webFrame := "event: turn.created\ndata: {\"turn_id\":\"web-1\"}\n\n"
	testFrame := "event: reply.started\ndata: {}\n\nevent: reply.delta\ndata: {\"text\":\"isolated test\"}\n\nevent: reply.completed\ndata: {\"text\":\"isolated test\"}\n\n"
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/voice/turns":
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, webFrame)
			w.(http.Flusher).Flush()
			select {
			case <-release:
				_, _ = io.WriteString(w, "event: turn.cancelled\ndata: {\"turn_id\":\"web-1\"}\n\n")
			case <-r.Context().Done():
			}
		case "/v1/voice/test/reply":
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, testFrame)
		case "/v1/transcribe":
			w.Header().Set("Content-Type", "application/json")
			_, _ = io.WriteString(w, `{"text":"isolated transcript"}`)
		case "/v1/voice/inspect":
			w.Header().Set("Content-Type", "application/json")
			_, _ = io.WriteString(w, voiceSnapshot)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	t.Cleanup(upstream.Close)
	public := httptest.NewServer(setupVoiceRouter(t, upstream.URL))
	t.Cleanup(public.Close)
	client := &http.Client{Timeout: 5 * time.Second}
	web, err := client.Post(public.URL+"/api/v1/voice/turns", "application/json", strings.NewReader(`{"text":"web question"}`))
	if err != nil {
		t.Fatal(err)
	}
	defer web.Body.Close()
	readExact(t, web.Body, webFrame)
	for _, tc := range []struct {
		path        string
		contentType string
		body        string
		want        string
	}{
		{"/test/reply", "application/json", `{"text":"isolated question"}`, testFrame},
		{"/transcribe", "audio/wav", "RIFFfixture", `{"text":"isolated transcript"}`},
	} {
		resp, err := client.Post(public.URL+"/api/v1/voice"+tc.path, tc.contentType, strings.NewReader(tc.body))
		if err != nil {
			t.Fatal(err)
		}
		body, _ := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		if resp.StatusCode != http.StatusOK || string(body) != tc.want || strings.Contains(string(body), "web-1") {
			t.Fatalf("test response not isolated: status=%d body=%q", resp.StatusCode, body)
		}
	}
	inspect, err := client.Get(public.URL + "/api/v1/voice/inspect")
	if err != nil {
		t.Fatal(err)
	}
	snapshot, _ := io.ReadAll(inspect.Body)
	_ = inspect.Body.Close()
	if string(snapshot) != voiceSnapshot || strings.Contains(string(snapshot), "isolated") {
		t.Fatalf("inspection changed: %s", snapshot)
	}
	unblock()
	rest, err := io.ReadAll(web.Body)
	if err != nil || string(rest) != "event: turn.cancelled\ndata: {\"turn_id\":\"web-1\"}\n\n" {
		t.Fatalf("isolated data entered web stream: %q err=%v", rest, err)
	}
}

func TestVoiceStreamDisconnectCancelsUpstreamRequest(t *testing.T) {
	for _, path := range []string{"/turns", "/test/reply"} {
		t.Run(path, func(t *testing.T) {
			cancelled := make(chan struct{})
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, ": ready\n\n")
				w.(http.Flusher).Flush()
				<-r.Context().Done()
				close(cancelled)
			}))
			t.Cleanup(upstream.Close)
			public := httptest.NewServer(setupVoiceRouter(t, upstream.URL))
			t.Cleanup(public.Close)
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			req, _ := http.NewRequestWithContext(ctx, "POST", public.URL+"/api/v1/voice"+path, strings.NewReader(`{"text":"question"}`))
			req.Header.Set("Content-Type", "application/json")
			client := &http.Client{Timeout: 5 * time.Second}
			resp, err := client.Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			readExact(t, resp.Body, ": ready\n\n")
			cancel()
			select {
			case <-cancelled:
			case <-time.After(2 * time.Second):
				t.Fatal("request cancellation did not reach Pheme")
			}
		})
	}
}

func TestVoiceTerminalErrorsAreForwardedWithoutRewriting(t *testing.T) {
	for _, event := range []string{
		"event: turn.failed\ndata: {\"turn_id\":\"web-1\",\"error\":{\"code\":\"stt_failed\",\"message\":\"no speech\"}}\n\n",
		"event: reply.failed\ndata: {\"error\":{\"code\":\"not_ready\",\"message\":\"reply unavailable\"}}\n\n",
		"event: turn.cancelled\ndata: {\"turn_id\":\"web-1\"}\n\n",
	} {
		upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, event)
		}))
		router := setupVoiceRouter(t, upstream.URL)
		resp := voiceRequest(router, "POST", "/test/reply", "application/json", `{"text":"question"}`)
		upstream.Close()
		if resp.Code != http.StatusOK || resp.Body.String() != event {
			t.Fatalf("Rust terminal event changed: status=%d body=%q", resp.Code, resp.Body.String())
		}
	}
}

func TestVoiceTransportFailuresDoNotLeakDetails(t *testing.T) {
	closed := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	closed.Close()
	resp := voiceRequest(setupVoiceRouter(t, closed.URL), "GET", "/inspect", "", "")
	assertSanitizedVoiceFailure(t, resp, closed.URL)

	for _, tc := range []struct {
		name    string
		path    string
		handler http.HandlerFunc
	}{
		{"invalid SSE response", "/turns", func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", "text/html")
			_, _ = io.WriteString(w, "private invalid response")
		}},
		{"truncated JSON", "/inspect", func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", "application/json")
			w.Header().Set("Content-Length", "100")
			_, _ = io.WriteString(w, "private truncated response")
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			upstream := httptest.NewServer(tc.handler)
			defer upstream.Close()
			method, contentType, body := "GET", "", ""
			if tc.path == "/turns" {
				method, contentType, body = "POST", "application/json", `{"text":"question"}`
			}
			resp := voiceRequest(setupVoiceRouter(t, upstream.URL), method, tc.path, contentType, body)
			assertSanitizedVoiceFailure(t, resp, upstream.URL)
		})
	}
}

func TestVoiceBrokenStreamDoesNotAppendTransportError(t *testing.T) {
	const prefix = "event: turn.created\ndata: {\"turn_id\":\"web-1\"}\n\n"
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.Header().Set("Content-Length", "1000")
		_, _ = io.WriteString(w, prefix)
		w.(http.Flusher).Flush()
	}))
	defer upstream.Close()
	resp := voiceRequest(setupVoiceRouter(t, upstream.URL), "POST", "/turns", "application/json", `{"text":"question"}`)
	if resp.Code != http.StatusOK || !resp.Flushed || resp.Body.String() != prefix {
		t.Fatalf("Go fabricated a stream error: status=%d body=%q", resp.Code, resp.Body.String())
	}
}

func TestVoiceGatewayTimeoutIsSanitized(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.GET("/api/v1/voice/inspect", func(c *gin.Context) {
		forwardVoice(c, nil, pheme.ErrTimeout)
	})
	resp := voiceRequest(router, "GET", "/inspect", "", "")
	if resp.Code != http.StatusGatewayTimeout || !strings.Contains(resp.Body.String(), `"code":"upstream_timeout"`) ||
		!strings.Contains(resp.Body.String(), `"message":"voice service timed out"`) {
		t.Fatalf("unexpected timeout: status=%d body=%q", resp.Code, resp.Body.String())
	}
}

func TestVoiceHasNoModelManagementRoutes(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { calls.Add(1) }))
	defer upstream.Close()
	router := setupVoiceRouter(t, upstream.URL)
	for _, path := range []string{"/models", "/models/catalog", "/models/download", "/models/select", "/downloads", "/management"} {
		for _, method := range []string{"GET", "POST"} {
			resp := voiceRequest(router, method, path, "application/json", `{}`)
			if resp.Code != http.StatusNotFound {
				t.Errorf("model route exists: %s %s status=%d", method, path, resp.Code)
			}
		}
	}
	if calls.Load() != 0 {
		t.Fatal("unknown routes reached Pheme")
	}
}

func setupVoiceRouter(t *testing.T, upstreamURL string) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	client, err := pheme.NewClient(upstreamURL)
	if err != nil {
		t.Fatal(err)
	}
	router := gin.New()
	NewVoiceHandler(service.NewVoiceService(client)).RegisterRoutes(router.Group("/api/v1/voice"))
	return router
}

func voiceRequest(router http.Handler, method, path, contentType, body string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, "/api/v1/voice"+path, strings.NewReader(body))
	req.Header.Set("Content-Type", contentType)
	resp := httptest.NewRecorder()
	router.ServeHTTP(resp, req)
	return resp
}

func readExact(t *testing.T, reader io.Reader, want string) {
	t.Helper()
	body := make([]byte, len(want))
	if _, err := io.ReadFull(reader, body); err != nil {
		t.Fatalf("stream did not flush promptly: %v; bytes=%q", err, body)
	}
	if !bytes.Equal(body, []byte(want)) {
		t.Fatalf("stream bytes changed: got %q want %q", body, want)
	}
}

func assertSanitizedVoiceFailure(t *testing.T, resp *httptest.ResponseRecorder, upstreamURL string) {
	t.Helper()
	if resp.Code != http.StatusBadGateway {
		t.Fatalf("transport failure status=%d, want 502; %s", resp.Code, resp.Body.String())
	}
	var body struct {
		Error struct {
			Code    string `json:"code"`
			Message string `json:"message"`
		} `json:"error"`
	}
	if err := json.Unmarshal(resp.Body.Bytes(), &body); err != nil || body.Error.Code != "upstream_unavailable" || body.Error.Message != "voice service unavailable" {
		t.Fatalf("unsanitized error body: %q err=%v", resp.Body.String(), err)
	}
	for _, detail := range []string{upstreamURL, "dial", "tcp", "private", "127.0.0.1"} {
		if strings.Contains(resp.Body.String(), detail) {
			t.Errorf("transport detail leaked: %q", detail)
		}
	}
}

type repeatedByteReader struct{}

func (repeatedByteReader) Read(buffer []byte) (int, error) {
	for i := range buffer {
		buffer[i] = 'x'
	}
	return len(buffer), nil
}
