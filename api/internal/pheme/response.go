package pheme

import (
	"context"
	"io"
	"net/http"
	"sync"
	"time"
)

// Response owns an upstream body and its request cancellation. Call Forward or
// Close once it is no longer needed. Neither JSON nor SSE events are rewritten.
type Response struct {
	status  int
	header  http.Header
	payload []byte
	stream  bool
	body    io.ReadCloser
	cancel  context.CancelFunc
	once    sync.Once
}

func (r *Response) Close() {
	r.once.Do(func() {
		r.cancel()
		_ = r.body.Close()
	})
}

func (r *Response) Forward(ctx context.Context, w http.ResponseWriter) error {
	defer r.Close()
	if ctx.Err() != nil {
		return ctx.Err()
	}
	for _, name := range []string{"Content-Type", "Retry-After"} {
		if value := r.header.Get(name); value != "" {
			w.Header().Set(name, value)
		}
	}
	w.Header().Set("Cache-Control", "no-store, no-transform")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	if !r.stream {
		w.WriteHeader(r.status)
		_, err := w.Write(r.payload)
		if err != nil {
			return transportError(ctx, err)
		}
		return nil
	}

	w.Header().Set("X-Accel-Buffering", "no")
	controller := http.NewResponseController(w)
	// A per-write deadline bounds slow clients without putting a total deadline
	// on review/generation. Some test writers do not support write deadlines.
	defer func() { _ = controller.SetWriteDeadline(time.Time{}) }()
	_ = controller.SetWriteDeadline(time.Now().Add(30 * time.Second))
	w.WriteHeader(r.status)
	if err := controller.Flush(); err != nil {
		return transportError(ctx, err)
	}

	buffer := make([]byte, 32<<10)
	for {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		n, readErr := r.body.Read(buffer)
		if n > 0 {
			_ = controller.SetWriteDeadline(time.Now().Add(30 * time.Second))
			if _, err := w.Write(buffer[:n]); err != nil {
				return transportError(ctx, err)
			}
			if err := controller.Flush(); err != nil {
				return transportError(ctx, err)
			}
		}
		if readErr == io.EOF {
			return nil
		}
		if readErr != nil {
			return transportError(ctx, readErr)
		}
		// Copy raw chunks: comments/heartbeats and split Unicode/event frames
		// remain intact. Adding our own frames here could corrupt a partial frame.
	}
}
