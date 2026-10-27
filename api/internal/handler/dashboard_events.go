package handler

import (
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"golang.org/x/net/websocket"
)

// DashboardEvents keeps the dashboard live. The server sends a lightweight
// refresh signal; the browser then reloads the current dashboard data through
// the normal HTTP API.
func DashboardEvents(c *gin.Context) {
	server := websocket.Server{
		Handshake: func(_ *websocket.Config, _ *http.Request) error { return nil },
		Handler: func(conn *websocket.Conn) {
			defer conn.Close()
			done := make(chan struct{})
			go func() {
				var message string
				for {
					if err := websocket.Message.Receive(conn, &message); err != nil {
						close(done)
						return
					}
				}
			}()
			ticker := time.NewTicker(2 * time.Second)
			defer ticker.Stop()
			for {
				select {
				case <-done:
					return
				case <-ticker.C:
					if err := websocket.Message.Send(conn, `{"type":"refresh"}`); err != nil {
						return
					}
				}
			}
		},
	}
	server.ServeHTTP(c.Writer, c.Request)
}
