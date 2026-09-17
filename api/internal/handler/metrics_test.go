package handler

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/gin-gonic/gin"
)

func TestMetricsHandlerAppendsAndReadsBatch(t *testing.T) {
	router, _ := setupMetricsRouter(t)
	body := bytes.NewBufferString(`{
		"schema_version":1,
		"run_id":"run-1",
		"sequence_number":0,
		"timestamp_ms":1000,
		"events":[{
			"schema_version":1,
			"run_id":"run-1",
			"sequence":0,
			"timestamp_ms":1000,
			"name":"end_to_end_request_duration_ms",
			"value":42.5,
			"unit":"milliseconds",
			"scope":"run",
			"source":"core.workflow"
		}]
	}`)
	request := httptest.NewRequest(http.MethodPost, "/api/v1/experiments/exp-1/metrics", body)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()

	router.ServeHTTP(response, request)

	if response.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d; body: %s", response.Code, http.StatusAccepted, response.Body.String())
	}

	request = httptest.NewRequest(http.MethodGet, "/api/v1/experiments/exp-1/metrics", nil)
	response = httptest.NewRecorder()
	router.ServeHTTP(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("GET status = %d, want %d; body: %s", response.Code, http.StatusOK, response.Body.String())
	}
	var payload struct {
		ExperimentID string          `json:"experiment_id"`
		Batches      []metrics.Batch `json:"batches"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatalf("failed to decode GET response: %v", err)
	}
	if payload.ExperimentID != "exp-1" || len(payload.Batches) != 1 {
		t.Fatalf("payload = %+v, want one exp-1 batch", payload)
	}
	if len(payload.Batches[0].Events) != 1 {
		t.Fatalf("event count = %d, want 1", len(payload.Batches[0].Events))
	}
}

func TestMetricsHandlerRejectsExperimentMismatch(t *testing.T) {
	router, _ := setupMetricsRouter(t)
	body := bytes.NewBufferString(`{
		"schema_version":1,
		"experiment_id":"exp-other",
		"run_id":"run-1",
		"sequence_number":0,
		"timestamp_ms":1000,
		"events":[]
	}`)
	request := httptest.NewRequest(http.MethodPost, "/api/v1/experiments/exp-1/metrics", body)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()

	router.ServeHTTP(response, request)

	if response.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d; body: %s", response.Code, http.StatusBadRequest, response.Body.String())
	}
}

func setupMetricsRouter(t *testing.T) (*gin.Engine, *metrics.Service) {
	t.Helper()

	gin.SetMode(gin.TestMode)
	repository := metrics.NewMemoryRepository()
	service := metrics.NewService(repository)
	handler := NewMetricsHandler(service)

	router := gin.New()
	router.POST("/api/v1/experiments/:id/metrics", handler.Append)
	router.GET("/api/v1/experiments/:id/metrics", handler.Get)

	return router, service
}
