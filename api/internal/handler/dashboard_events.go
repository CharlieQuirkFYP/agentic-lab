package handler

import (
	"encoding/json"
	"net/http"
	"sync"

	"github.com/gin-gonic/gin"
	"golang.org/x/net/websocket"
)

type DashboardEventHub struct {
	mu      sync.Mutex
	clients map[*websocket.Conn]struct{}
}

func NewDashboardEventHub() *DashboardEventHub {
	return &DashboardEventHub{clients: make(map[*websocket.Conn]struct{})}
}

func (h *DashboardEventHub) Handler(c *gin.Context) {
	websocket.Server{
		Handshake: func(_ *websocket.Config, _ *http.Request) error { return nil },
		Handler: func(conn *websocket.Conn) {
			h.mu.Lock()
			h.clients[conn] = struct{}{}
			h.mu.Unlock()
			defer func() {
				h.mu.Lock()
				delete(h.clients, conn)
				h.mu.Unlock()
				_ = conn.Close()
			}()

			var message string
			for websocket.Message.Receive(conn, &message) == nil {
			}
		},
	}.ServeHTTP(c.Writer, c.Request)
}

func (h *DashboardEventHub) Publish(event string) {
	payload, _ := json.Marshal(map[string]string{"type": event})
	h.mu.Lock()
	defer h.mu.Unlock()
	for conn := range h.clients {
		if err := websocket.Message.Send(conn, string(payload)); err != nil {
			delete(h.clients, conn)
			_ = conn.Close()
		}
	}
}
