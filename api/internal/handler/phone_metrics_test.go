package handler

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
	"github.com/gin-gonic/gin"
)

const phoneFixture = `{"schema_version":1,"device":{"installation_id":"fixture","hardware":"test-fixture-not-a-phone","os_version":"test","app_version":"test","build":"test"},"batch":{"schema_version":1,"run_id":"fixture-run","sequence_number":0,"timestamp_ms":1000,"events":[{"schema_version":1,"run_id":"fixture-run","sequence":0,"timestamp_ms":1000,"name":"reasoning_duration_ms","value":42,"unit":"milliseconds","scope":"run","source":"integration_fixture"}]}}`

func TestPhoneMetricsHTTP(t *testing.T) {
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	gin.SetMode(gin.TestMode)
	router := gin.New()
	cors, err := NewBenchmarkCORS([]string{"http://localhost:5173"})
	if err != nil {
		t.Fatal(err)
	}
	notifications := 0
	NewPhoneMetricsHandler(phonemetrics.NewStore(db), func() { notifications++ }).RegisterRoutes(router.Group("/api/v1/phone", cors))
	send := func(method, path, body string) *httptest.ResponseRecorder {
		request := httptest.NewRequest(method, path, strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		response := httptest.NewRecorder()
		router.ServeHTTP(response, request)
		return response
	}
	for range 2 {
		response := send(http.MethodPost, "/api/v1/phone/metrics", phoneFixture)
		if response.Code != 202 || !strings.Contains(response.Body.String(), `"status":"accepted"`) {
			t.Fatalf("upload: %d %s", response.Code, response.Body.String())
		}
	}
	if notifications != 2 {
		t.Fatal("uploads did not notify dashboard")
	}
	response := send(http.MethodGet, "/api/v1/phone/metrics?limit=100", "")
	var page phonemetrics.Page
	if err := json.Unmarshal(response.Body.Bytes(), &page); err != nil || page.Total != 1 || page.Source != phonemetrics.Source {
		t.Fatalf("read: %d %+v %v", response.Code, page, err)
	}
	if page.Batches[0].ExperimentID == "" || page.Batches[0].Events[0].ExperimentID != page.Batches[0].ExperimentID {
		t.Fatal("missing server-assigned correlation")
	}
	changed := strings.Replace(phoneFixture, `"value":42`, `"value":43`, 1)
	if response := send(http.MethodPost, "/api/v1/phone/metrics", changed); response.Code != http.StatusConflict {
		t.Fatalf("conflict = %d", response.Code)
	}
	for _, body := range []string{"{}", "null", strings.Replace(phoneFixture, `"sequence_number":0,`, "", 1)} {
		if response := send(http.MethodPost, "/api/v1/phone/metrics", body); response.Code != http.StatusBadRequest {
			t.Fatalf("invalid body = %d", response.Code)
		}
	}
	for _, query := range []string{"limit=101", "unknown=1", "limit=1&limit=2", "offset=-1"} {
		if response := send(http.MethodGet, "/api/v1/phone/metrics?"+query, ""); response.Code != http.StatusBadRequest {
			t.Fatalf("invalid query %q = %d", query, response.Code)
		}
	}
	if response := send(http.MethodPost, "/api/v1/phone/metrics", string(bytes.Repeat([]byte(" "), 4<<20+1))); response.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("oversize = %d", response.Code)
	}
	request := httptest.NewRequest(http.MethodOptions, "/api/v1/phone/metrics", nil)
	request.Header.Set("Origin", "http://localhost:5173")
	request.Header.Set("Access-Control-Request-Method", "POST")
	request.Header.Set("Access-Control-Request-Headers", "Content-Type")
	response = httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusNoContent || response.Header().Get("Access-Control-Allow-Origin") == "" {
		t.Fatalf("preflight = %d", response.Code)
	}
	request = httptest.NewRequest(http.MethodPost, "/api/v1/phone/metrics", strings.NewReader(phoneFixture))
	request.Header.Set("Origin", "http://untrusted.example")
	response = httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden {
		t.Fatalf("untrusted browser origin = %d", response.Code)
	}
	var count int
	_ = db.QueryRow("SELECT COUNT(*) FROM metric_batches").Scan(&count)
	if count != 1 {
		t.Fatal(metrics.ErrBatchConflict)
	}
}
