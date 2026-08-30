package handler

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/gin-gonic/gin"
)

func TestExperimentHandlerCreateReturnsAccepted(t *testing.T) {
	router, _ := setupExperimentRouter(t)

	body := bytes.NewBufferString(`{"use_case":"incident-reporting","dataset":"sample"}`)
	req := httptest.NewRequest(http.MethodPost, "/api/v1/experiments", body)
	req.Header.Set("Content-Type", "application/json")
	resp := httptest.NewRecorder()

	router.ServeHTTP(resp, req)

	if resp.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d; body: %s", resp.Code, http.StatusAccepted, resp.Body.String())
	}

	var response benchmark.CreateExperimentResponse
	if err := json.Unmarshal(resp.Body.Bytes(), &response); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if response.ID == "" {
		t.Fatal("response ID is empty")
	}
	if response.Status != benchmark.ExperimentStatusQueued {
		t.Fatalf("status = %q, want %q", response.Status, benchmark.ExperimentStatusQueued)
	}
}

func TestExperimentHandlerCreateRequiresUseCase(t *testing.T) {
	router, _ := setupExperimentRouter(t)

	req := httptest.NewRequest(http.MethodPost, "/api/v1/experiments", bytes.NewBufferString(`{}`))
	req.Header.Set("Content-Type", "application/json")
	resp := httptest.NewRecorder()

	router.ServeHTTP(resp, req)

	if resp.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d; body: %s", resp.Code, http.StatusBadRequest, resp.Body.String())
	}
}

func TestExperimentHandlerCreateRejectsUnsupportedUseCase(t *testing.T) {
	router, _ := setupExperimentRouter(t)

	body := bytes.NewBufferString(`{"use_case":"unsupported"}`)
	req := httptest.NewRequest(http.MethodPost, "/api/v1/experiments", body)
	req.Header.Set("Content-Type", "application/json")
	resp := httptest.NewRecorder()

	router.ServeHTTP(resp, req)

	if resp.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d; body: %s", resp.Code, http.StatusBadRequest, resp.Body.String())
	}
}

func TestExperimentHandlerGetExistingExperiment(t *testing.T) {
	router, service := setupExperimentRouter(t)
	experiment, err := service.CreateExperiment(t.Context(), benchmark.CreateExperimentRequest{
		UseCase: benchmark.UseCaseInterviewAssistant,
	})
	if err != nil {
		t.Fatalf("CreateExperiment returned error: %v", err)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/v1/experiments/"+experiment.ID, nil)
	resp := httptest.NewRecorder()

	router.ServeHTTP(resp, req)

	if resp.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d; body: %s", resp.Code, http.StatusOK, resp.Body.String())
	}

	var response benchmark.Experiment
	if err := json.Unmarshal(resp.Body.Bytes(), &response); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if response.ID != experiment.ID {
		t.Fatalf("ID = %q, want %q", response.ID, experiment.ID)
	}
}

func TestExperimentHandlerGetUnknownExperiment(t *testing.T) {
	router, _ := setupExperimentRouter(t)

	req := httptest.NewRequest(http.MethodGet, "/api/v1/experiments/unknown", nil)
	resp := httptest.NewRecorder()

	router.ServeHTTP(resp, req)

	if resp.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want %d; body: %s", resp.Code, http.StatusNotFound, resp.Body.String())
	}
}

func setupExperimentRouter(t *testing.T) (*gin.Engine, *benchmark.Service) {
	t.Helper()

	gin.SetMode(gin.TestMode)
	repository := benchmark.NewMemoryRepository()
	queue, err := benchmark.NewJobQueue(10)
	if err != nil {
		t.Fatalf("NewJobQueue returned error: %v", err)
	}
	service := benchmark.NewService(repository, queue, benchmark.NewMockRunner())
	experimentHandler := NewExperimentHandler(service)

	router := gin.New()
	router.POST("/api/v1/experiments", experimentHandler.Create)
	router.GET("/api/v1/experiments/:id", experimentHandler.Get)

	return router, service
}
