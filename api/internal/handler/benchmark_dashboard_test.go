package handler

import (
	"bytes"
	"encoding/csv"
	"encoding/json"
	"math"
	"net/http"
	"net/http/httptest"
	"net/url"
	"path/filepath"
	"strings"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
	"github.com/gin-gonic/gin"
)

func dashboardRouter(t *testing.T, path string) (*gin.Engine, *benchmark.Service, func()) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	db, err := storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	queue, err := benchmark.NewJobQueue(100)
	if err != nil {
		t.Fatal(err)
	}
	service := benchmark.NewService(benchmark.NewSQLiteRepository(db), queue, benchmark.NewMockRunner())
	measurements := metrics.NewService(metrics.NewSQLiteRepository(db))
	h := NewBenchmarkDashboardHandler(service, measurements)
	cors, err := NewBenchmarkCORS([]string{"http://localhost:5173"})
	if err != nil {
		t.Fatal(err)
	}
	router := gin.New()
	h.RegisterRoutes(router.Group("/api/v1/experiments", cors), measurements)
	return router, service, func() {
		if err := db.Close(); err != nil {
			t.Error(err)
		}
	}
}

func dashboardRequest(t *testing.T, router http.Handler, method, path string, body any, want int) *httptest.ResponseRecorder {
	t.Helper()
	var encoded []byte
	if body != nil {
		var err error
		encoded, err = json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
	}
	req := httptest.NewRequest(method, path, bytes.NewReader(encoded))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://localhost:5173")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if response.Code != want {
		t.Fatalf("%s %s: status=%d want=%d body=%s", method, path, response.Code, want, response.Body.String())
	}
	return response
}

func decodeDashboard[T any](t *testing.T, response *httptest.ResponseRecorder) T {
	t.Helper()
	var result T
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatalf("decode %s: %v", response.Body.String(), err)
	}
	return result
}

func dashboardBatch(run string, seq int64, value any) metrics.Batch {
	e := metrics.Event{SchemaVersion: 1, RunID: run, Sequence: seq, TimestampMS: 1000 + seq, Name: "latency", Value: value, Unit: "milliseconds", Scope: "run", Source: "fixture"}
	if value == nil {
		e.UnavailableReason = "sensor unavailable"
	}
	return metrics.Batch{SchemaVersion: 1, RunID: run, SequenceNumber: seq, TimestampMS: 1000 + seq, Events: []metrics.Event{e}}
}

func TestDashboardPersistentFrontendFlow(t *testing.T) {
	path := filepath.Join(t.TempDir(), "experiments.sqlite")
	router, service, closeDB := dashboardRouter(t, path)
	created := decodeDashboard[benchmark.CreateExperimentResponse](t, dashboardRequest(t, router, "POST", "/api/v1/experiments", map[string]any{"use_case": "incident-reporting", "dataset": "local-v1", "model_config": map[string]string{"name": "=dangerous", "version": "v1"}}, 202))
	base := "/api/v1/experiments/" + created.ID
	queued := decodeDashboard[benchmark.Experiment](t, dashboardRequest(t, router, "GET", base, nil, 200))
	if created.Status != "queued" || queued.ResultSource != "mock" {
		t.Fatalf("unexpected create: %+v %+v", created, queued)
	}
	empty := decodeDashboard[benchmark.Summary](t, dashboardRequest(t, router, "GET", base+"/summary", nil, 200))
	if empty.Result != nil || len(empty.Metrics) != 0 || empty.Metrics == nil || empty.RunCount != 0 {
		t.Fatalf("queued summary=%+v", empty)
	}
	if err := service.ProcessExperiment(t.Context(), created.ID); err != nil {
		t.Fatal(err)
	}
	for _, batch := range []metrics.Batch{dashboardBatch("run-b", 0, 30.0), dashboardBatch("run-a", 0, 10.0), dashboardBatch("run-a", 1, nil), dashboardBatch("run-a", 2, 20.0)} {
		dashboardRequest(t, router, "POST", base+"/metrics", batch, 202)
		// A retry is accepted but must not contribute a second observation.
		dashboardRequest(t, router, "POST", base+"/metrics", batch, 202)
	}
	dashboardRequest(t, router, "POST", base+"/metrics", dashboardBatch("run-a", 0, 99.0), 409)
	// Keep process/system observations separate even when their names match.
	batch := dashboardBatch("run-a", 3, 100.0)
	batch.Events[0].Scope = "system"
	dashboardRequest(t, router, "POST", base+"/metrics", batch, 202)
	batch = dashboardBatch("run-a", 4, "success")
	batch.Events[0].Name = "workflow_outcome"
	batch.Events[0].Unit = "status"
	dashboardRequest(t, router, "POST", base+"/metrics", batch, 202)
	batch = dashboardBatch("run-a", 5, nil)
	batch.Events[0].Name = "whole_device_power_watts"
	batch.Events[0].Unit = "watts"
	dashboardRequest(t, router, "POST", base+"/metrics", batch, 202)
	closeDB()
	router, _, closeDB = dashboardRouter(t, path)
	defer closeDB()
	page := decodeDashboard[benchmark.ExperimentPage](t, dashboardRequest(t, router, "GET", "/api/v1/experiments?status=completed&dataset=local-v1&model="+url.QueryEscape("=dangerous"), nil, 200))
	if page.Total != 1 || len(page.Experiments) != 1 || page.Experiments[0].ID != created.ID {
		t.Fatalf("history=%+v", page)
	}
	summary := decodeDashboard[benchmark.Summary](t, dashboardRequest(t, router, "GET", base+"/summary", nil, 200))
	if summary.Status != "completed" || summary.ResultSource != "mock" || summary.Result == nil || summary.Result.LatencyMS != 123 || summary.RunCount != 2 {
		t.Fatalf("summary=%+v", summary)
	}
	var found bool
	for _, metric := range summary.Metrics {
		if metric.Name == "latency" && metric.Scope == "run" {
			found = true
			if metric.ObservationCount != 4 || metric.NumericCount != 3 || metric.UnavailableCount != 1 || metric.Mean == nil || *metric.Mean != 20 || *metric.Min != 10 || *metric.Max != 30 {
				t.Fatalf("latency=%+v", metric)
			}
		}
		if metric.Name == "whole_device_power_watts" && (metric.Mean != nil || metric.Min != nil || metric.Max != nil || len(metric.UnavailableReasons) != 1) {
			t.Fatalf("unavailable power=%+v", metric)
		}
		if metric.Name == "workflow_outcome" && (metric.NumericCount != 0 || metric.UnavailableCount != 0 || metric.Mean != nil) {
			t.Fatalf("string event=%+v", metric)
		}
	}
	if !found {
		t.Fatal("missing latency")
	}
	runs := decodeDashboard[benchmark.RunPage](t, dashboardRequest(t, router, "GET", base+"/runs?limit=1&offset=1", nil, 200))
	if runs.Total != 2 || len(runs.Runs) != 1 || runs.Runs[0].RunID != "run-b" || runs.Runs[0].EventCount != 1 {
		t.Fatalf("runs=%+v", runs)
	}
	measurements := decodeDashboard[metrics.Page](t, dashboardRequest(t, router, "GET", base+"/metrics?run_id=run-a&name=latency&limit=1&offset=1", nil, 200))
	if measurements.Total != 4 || len(measurements.Batches) != 1 || measurements.Batches[0].SequenceNumber != 1 || measurements.Batches[0].Events[0].Value != nil {
		t.Fatalf("metrics=%+v", measurements)
	}
	legacy := decodeDashboard[metrics.Page](t, dashboardRequest(t, router, "GET", base+"/metrics", nil, 200))
	if len(legacy.Batches) != 7 || legacy.Total != 7 || legacy.Limit != 0 {
		t.Fatalf("legacy metrics=%+v", legacy)
	}
	exported := decodeDashboard[benchmark.Export](t, dashboardRequest(t, router, "GET", base+"/export?format=json", nil, 200))
	if exported.SchemaVersion != 1 || len(exported.Batches) != 7 || exported.Experiment.ModelConfig.Version != "v1" {
		t.Fatalf("export=%+v", exported)
	}
	csvResponse := dashboardRequest(t, router, "GET", base+"/export?format=csv", nil, 200)
	rows, err := csv.NewReader(csvResponse.Body).ReadAll()
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 8 || rows[1][3] != "'=dangerous" || !strings.Contains(csvResponse.Header().Get("Content-Disposition"), "experiment.csv") {
		t.Fatalf("CSV=%v headers=%v", rows, csvResponse.Header())
	}
}

func TestDashboardEmptyAndErrorContracts(t *testing.T) {
	router, service, closeDB := dashboardRouter(t, ":memory:")
	defer closeDB()
	empty := dashboardRequest(t, router, "GET", "/api/v1/experiments", nil, 200)
	if !strings.Contains(empty.Body.String(), `"experiments":[]`) {
		t.Fatalf("empty history=%s", empty.Body.String())
	}
	e, err := service.CreateExperiment(t.Context(), benchmark.CreateExperimentRequest{UseCase: benchmark.UseCaseIncidentReporting})
	if err != nil {
		t.Fatal(err)
	}
	base := "/api/v1/experiments/" + e.ID
	for _, path := range []string{base + "/runs", base + "/metrics"} {
		response := dashboardRequest(t, router, "GET", path, nil, 200)
		if !strings.Contains(response.Body.String(), "[]") {
			t.Fatalf("empty list=%s", response.Body.String())
		}
	}
	for _, suffix := range []string{"", "/summary", "/runs", "/metrics", "/export"} {
		dashboardRequest(t, router, "GET", "/api/v1/experiments/missing"+suffix, nil, 404)
	}
	dashboardRequest(t, router, "POST", "/api/v1/experiments/missing/metrics", dashboardBatch("run-a", 0, 1.0), 404)
	for _, query := range []string{"limit=0", "limit=101", "offset=-1", "limit=abc", "limit=1&limit=2", "limit=", "limit=%ZZ", "status=invalid", "use_case=invalid", "created_after=bad", "created_after=2026-10-20T00:00:00Z&created_before=2026-10-19T00:00:00Z", "sort=unknown"} {
		dashboardRequest(t, router, "GET", "/api/v1/experiments?"+query, nil, 400)
	}
	for _, suffix := range []string{"/runs?limit=0", "/runs?offset=-1", "/metrics?offset=1", "/metrics?limit=101", "/metrics?name=", "/summary?unknown=1", "/export?format=xml"} {
		dashboardRequest(t, router, "GET", base+suffix, nil, 400)
	}
	for _, query := range []string{"limit=1&offset=999999999", "model=no-match"} {
		page := decodeDashboard[benchmark.ExperimentPage](t, dashboardRequest(t, router, "GET", "/api/v1/experiments?"+query, nil, 200))
		if len(page.Experiments) != 0 || page.Experiments == nil {
			t.Fatalf("empty page=%+v", page)
		}
	}
	response := dashboardRequest(t, router, "GET", base+"/export?format=csv", nil, 200)
	rows, err := csv.NewReader(response.Body).ReadAll()
	if err != nil || len(rows) != 2 {
		t.Fatalf("empty CSV=%v err=%v", rows, err)
	}
	bad := dashboardBatch("run-a", 0, nil)
	bad.Events[0].UnavailableReason = ""
	dashboardRequest(t, router, "POST", base+"/metrics", bad, 400)
	bad = dashboardBatch("run-a", 0, 1)
	bad.ExperimentID = "other"
	dashboardRequest(t, router, "POST", base+"/metrics", bad, 400)
	bad = dashboardBatch("run-a", 0, 1)
	bad.Events[0].ExperimentID = "other"
	dashboardRequest(t, router, "POST", base+"/metrics", bad, 400)
	dashboardRequest(t, router, "POST", base+"/metrics", dashboardBatch("oversized", 0, strings.Repeat("x", 4<<20)), 413)
	// Ensure extreme valid numbers still serialize a finite mean.
	for i, value := range []float64{math.MaxFloat64, -math.MaxFloat64} {
		dashboardRequest(t, router, "POST", base+"/metrics", dashboardBatch("large", int64(i), value), 202)
	}
	summary := decodeDashboard[benchmark.Summary](t, dashboardRequest(t, router, "GET", base+"/summary", nil, 200))
	if summary.Metrics[0].Mean == nil || *summary.Metrics[0].Mean != 0 {
		t.Fatalf("extreme mean=%+v", summary.Metrics)
	}
}

func TestDashboardBrowserAccess(t *testing.T) {
	router, _, closeDB := dashboardRouter(t, ":memory:")
	defer closeDB()
	for _, tc := range []struct {
		method, path, origin, requestMethod string
		want                                int
	}{
		{"OPTIONS", "/api/v1/experiments", "http://localhost:5173", "POST", 204},
		{"OPTIONS", "/api/v1/experiments/missing/metrics", "http://localhost:5173", "POST", 204},
		{"OPTIONS", "/api/v1/experiments/missing/export", "http://localhost:5173", "GET", 204},
		{"GET", "/api/v1/experiments", "http://localhost:5173", "", 200},
		{"GET", "/api/v1/experiments", "https://untrusted.example", "", 403},
		{"GET", "/api/v1/experiments", "", "", 200},
	} {
		req := httptest.NewRequest(tc.method, tc.path, nil)
		req.Header.Set("Origin", tc.origin)
		req.Header.Set("Access-Control-Request-Method", tc.requestMethod)
		req.Header.Set("Access-Control-Request-Headers", "Content-Type")
		resp := httptest.NewRecorder()
		router.ServeHTTP(resp, req)
		if resp.Code != tc.want {
			t.Fatalf("%+v status=%d body=%s", tc, resp.Code, resp.Body.String())
		}
		if tc.origin == "http://localhost:5173" && resp.Header().Get("Access-Control-Allow-Origin") != tc.origin {
			t.Fatal("missing browser CORS")
		}
		if tc.method == "GET" && tc.origin == "http://localhost:5173" && !strings.Contains(resp.Header().Get("Access-Control-Expose-Headers"), "Content-Disposition") {
			t.Fatal("download filename is not exposed")
		}
	}
}
