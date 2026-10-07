package pheme

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestClientURLValidation(t *testing.T) {
	for _, address := range []string{"", "127.0.0.1:8000", "/relative", "file:///tmp/pheme", "http://", "http://user:secret@localhost", "http://localhost?x=y", "http://localhost?", "http://localhost/#fragment", "http://[invalid"} {
		t.Run(address, func(t *testing.T) {
			if _, err := NewClient(address); !errors.Is(err, ErrInvalidURL) {
				t.Fatalf("NewClient(%q) error = %v, want ErrInvalidURL", address, err)
			}
		})
	}
	for _, address := range []string{DefaultURL, "https://localhost:8000/pheme/"} {
		client, err := NewClient(address)
		if err != nil {
			t.Fatal(err)
		}
		if client.httpClient.Timeout != 0 {
			t.Fatal("SSE client must not have a total timeout")
		}
		transport := client.httpClient.Transport.(*http.Transport)
		if transport.ResponseHeaderTimeout <= 0 {
			t.Fatal("upstream header timeout must be bounded")
		}
	}
}

func TestClientEscapesOpaqueTurnIDsAndRejectsTraversal(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.EscapedPath() != "/prefix/v1/voice/turns/turn%20%3F%23%C3%A9" || r.URL.RawQuery != "" {
			t.Errorf("unexpected upstream URL: %s", r.URL)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"status":"awaiting_review"}`)
	}))
	defer upstream.Close()
	client, err := NewClient(upstream.URL + "/prefix/")
	if err != nil {
		t.Fatal(err)
	}
	response, err := client.TurnStatus(t.Context(), "turn ?#é")
	if err != nil {
		t.Fatal(err)
	}
	response.Close()
	for _, id := range []string{"", ".", "..", "../reset", "a/b", "a\\b", "a\n", strings.Repeat("x", 257)} {
		if _, err := client.TurnStatus(t.Context(), id); !errors.Is(err, ErrInvalidTurnID) {
			t.Errorf("TurnStatus(%q) error = %v, want ErrInvalidTurnID", id, err)
		}
	}
	if calls.Load() != 1 {
		t.Fatalf("upstream calls = %d, want 1", calls.Load())
	}
}

func TestClientHeaderTimeoutIsSanitized(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(_ http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
	}))
	defer upstream.Close()
	client, err := NewClient(upstream.URL)
	if err != nil {
		t.Fatal(err)
	}
	client.httpClient.Transport.(*http.Transport).ResponseHeaderTimeout = 30 * time.Millisecond
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Second)
	defer cancel()
	if _, err := client.Inspect(ctx); !errors.Is(err, ErrTimeout) || strings.Contains(err.Error(), upstream.URL) {
		t.Fatalf("Inspect error = %v, want sanitized timeout", err)
	}
}

func TestClientHeaderTimeoutDoesNotBoundReviewStream(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, ": ready\n\n")
		w.(http.Flusher).Flush()
		select {
		case <-time.After(100 * time.Millisecond):
			_, _ = io.WriteString(w, "event: reply.completed\ndata: {\"text\":\"done\"}\n\n")
		case <-r.Context().Done():
		}
	}))
	defer upstream.Close()
	client, err := NewClient(upstream.URL)
	if err != nil {
		t.Fatal(err)
	}
	client.httpClient.Transport.(*http.Transport).ResponseHeaderTimeout = 30 * time.Millisecond
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Second)
	defer cancel()
	response, err := client.StartTurn(ctx, Input{Body: []byte(`{"text":"question"}`), ContentType: "application/json"})
	if err != nil {
		t.Fatal(err)
	}
	recorder := httptest.NewRecorder()
	if err := response.Forward(ctx, recorder); err != nil {
		t.Fatal(err)
	}
	if !recorder.Flushed || !strings.Contains(recorder.Body.String(), "reply.completed") {
		t.Fatalf("review stream not completed: %s", recorder.Body.String())
	}
}

func TestClientDoesNotFollowRedirects(t *testing.T) {
	var redirected atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/management" {
			redirected.Add(1)
		}
		w.Header().Set("Location", "/management")
		w.WriteHeader(http.StatusTemporaryRedirect)
	}))
	defer upstream.Close()
	client, err := NewClient(upstream.URL)
	if err != nil {
		t.Fatal(err)
	}
	response, err := client.Reset(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	recorder := httptest.NewRecorder()
	if err := response.Forward(t.Context(), recorder); err != nil {
		t.Fatal(err)
	}
	if recorder.Code != http.StatusTemporaryRedirect || redirected.Load() != 0 || recorder.Header().Get("Location") != "" {
		t.Fatalf("redirect was followed or leaked: status=%d calls=%d headers=%v", recorder.Code, redirected.Load(), recorder.Header())
	}
}
