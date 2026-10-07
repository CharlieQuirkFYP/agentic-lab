package pheme

import (
	"bytes"
	"context"
	"errors"
	"io"
	"mime"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode"
)

const DefaultURL = "http://127.0.0.1:8000"

var (
	ErrInvalidURL      = errors.New("invalid Pheme VA URL")
	ErrInvalidTurnID   = errors.New("invalid turn ID")
	ErrUnavailable     = errors.New("Pheme VA unavailable")
	ErrTimeout         = errors.New("Pheme VA timed out")
	ErrInvalidResponse = errors.New("invalid Pheme VA response")
)

// Input is transport data, not a second copy of Pheme's voice message schema.
// Body is forwarded verbatim, including whitespace and unknown JSON fields.
type Input struct {
	Body           []byte
	ContentType    string
	IdempotencyKey string
}

type Client struct {
	baseURL    *url.URL
	httpClient *http.Client
}

func NewClient(baseURL string) (*Client, error) {
	u, err := url.Parse(baseURL)
	if err != nil || u.Hostname() == "" || (u.Scheme != "http" && u.Scheme != "https") ||
		u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.Opaque != "" {
		return nil, ErrInvalidURL
	}
	u.Path = strings.TrimRight(u.Path, "/")
	u.RawPath = strings.TrimRight(u.RawPath, "/")
	transport := http.DefaultTransport.(*http.Transport).Clone()
	// Stateless STT may legitimately take the Rust host's full 180-second
	// default deadline before sending headers. SSE starts promptly but has no
	// total deadline because it includes the human review interval.
	transport.ResponseHeaderTimeout = 210 * time.Second
	transport.DisableCompression = true
	return &Client{
		baseURL: u,
		httpClient: &http.Client{
			Transport: transport,
			// No total timeout: a stream includes Pheme's bounded human review wait.
			CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}, nil
}

func (c *Client) StartTurn(ctx context.Context, input Input) (*Response, error) {
	return c.request(ctx, http.MethodPost, []string{"v1", "voice", "turns"}, input, true)
}

func (c *Client) Submit(ctx context.Context, turnID string, input Input) (*Response, error) {
	return c.turnRequest(ctx, http.MethodPost, turnID, "submit", input)
}

func (c *Client) TurnStatus(ctx context.Context, turnID string) (*Response, error) {
	return c.turnRequest(ctx, http.MethodGet, turnID, "", Input{})
}

func (c *Client) Cancel(ctx context.Context, turnID string) (*Response, error) {
	return c.turnRequest(ctx, http.MethodPost, turnID, "cancel", Input{})
}

func (c *Client) Reset(ctx context.Context) (*Response, error) {
	return c.request(ctx, http.MethodPost, []string{"v1", "voice", "reset"}, Input{}, false)
}

func (c *Client) Inspect(ctx context.Context) (*Response, error) {
	return c.request(ctx, http.MethodGet, []string{"v1", "voice", "inspect"}, Input{}, false)
}

func (c *Client) TestReply(ctx context.Context, input Input) (*Response, error) {
	return c.request(ctx, http.MethodPost, []string{"v1", "voice", "test", "reply"}, input, true)
}

func (c *Client) Transcribe(ctx context.Context, input Input) (*Response, error) {
	return c.request(ctx, http.MethodPost, []string{"v1", "transcribe"}, input, false)
}

func (c *Client) turnRequest(ctx context.Context, method, turnID, action string, input Input) (*Response, error) {
	if turnID == "" || len(turnID) > 256 || turnID == "." || turnID == ".." ||
		strings.ContainsAny(turnID, "/\\") || strings.ContainsFunc(turnID, unicode.IsControl) {
		return nil, ErrInvalidTurnID
	}
	segments := []string{"v1", "voice", "turns", turnID}
	if action != "" {
		segments = append(segments, action)
	}
	return c.request(ctx, method, segments, input, false)
}

func (c *Client) request(ctx context.Context, method string, segments []string, input Input, stream bool) (*Response, error) {
	u := *c.baseURL
	escapedPath := c.baseURL.EscapedPath()
	for _, segment := range segments {
		u.Path += "/" + segment
		escapedPath += "/" + url.PathEscape(segment)
	}
	u.RawPath = escapedPath

	requestCtx, cancel := context.WithCancel(ctx)
	req, err := http.NewRequestWithContext(requestCtx, method, u.String(), bytes.NewReader(input.Body))
	if err != nil {
		cancel()
		return nil, ErrInvalidResponse
	}
	if input.ContentType != "" {
		req.Header.Set("Content-Type", input.ContentType)
	}
	if input.IdempotencyKey != "" {
		req.Header.Set("Idempotency-Key", input.IdempotencyKey)
	}
	req.Header.Set("Accept-Encoding", "identity")
	if stream {
		req.Header.Set("Accept", "text/event-stream")
	} else {
		req.Header.Set("Accept", "application/json")
	}

	upstream, err := c.httpClient.Do(req)
	if err != nil {
		cancel()
		return nil, transportError(ctx, err)
	}
	response := &Response{
		status: upstream.StatusCode,
		header: upstream.Header,
		body:   upstream.Body,
		cancel: cancel,
	}
	if stream && upstream.StatusCode >= 200 && upstream.StatusCode < 300 {
		contentType, _, err := mime.ParseMediaType(upstream.Header.Get("Content-Type"))
		if err == nil && contentType == "text/event-stream" {
			response.stream = true
			return response, nil
		}
		// Pheme may acknowledge a known retry with JSON rather than opening
		// another stream. The upstream status/result remains authoritative.
		if upstream.StatusCode != http.StatusNoContent && (err != nil || contentType != "application/json") {
			response.Close()
			return nil, ErrInvalidResponse
		}
	}

	// Buffer only non-stream responses so a truncated/oversized upstream JSON
	// response can fail before committing its status to the public connection.
	const maxResponseBytes = 32 << 20
	response.payload, err = io.ReadAll(io.LimitReader(upstream.Body, maxResponseBytes+1))
	response.Close()
	if err != nil {
		return nil, transportError(ctx, err)
	}
	if len(response.payload) > maxResponseBytes {
		return nil, ErrInvalidResponse
	}
	return response, nil
}

func transportError(ctx context.Context, err error) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}
	var networkError net.Error
	if errors.As(err, &networkError) && networkError.Timeout() {
		return ErrTimeout
	}
	// Never return the URL, host, or dial/TLS details from net/http's errors.
	return ErrUnavailable
}
