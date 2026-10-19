package handler

import (
	"bytes"
	"encoding/csv"
	"encoding/json"
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/gin-gonic/gin"
)

type BenchmarkDashboardHandler struct {
	experiments *benchmark.Service
	dashboard   *benchmark.DashboardService
}

func NewBenchmarkDashboardHandler(experiments *benchmark.Service, measurements *metrics.Service) *BenchmarkDashboardHandler {
	return &BenchmarkDashboardHandler{experiments: experiments, dashboard: benchmark.NewDashboardService(experiments, measurements)}
}

// RegisterRoutes is shared by the real server and dashboard contract tests.
func (h *BenchmarkDashboardHandler) RegisterRoutes(group *gin.RouterGroup, measurements *metrics.Service) {
	experiments := NewExperimentHandler(h.experiments)
	metricHandler := NewMetricsHandler(measurements)
	group.POST("", experiments.Create)
	group.GET("", h.List)
	group.OPTIONS("", func(c *gin.Context) { c.Status(http.StatusNoContent) })
	detail := group.Group("/:id", h.requireExperiment)
	detail.GET("", experiments.Get)
	detail.GET("/summary", h.Summary)
	detail.GET("/runs", h.Runs)
	detail.GET("/metrics", metricHandler.Get)
	detail.POST("/metrics", metricHandler.Append)
	detail.GET("/export", h.Export)
	// Preflights must not depend on whether an experiment currently exists.
	for _, path := range []string{"/:id", "/:id/summary", "/:id/runs", "/:id/metrics", "/:id/export"} {
		group.OPTIONS(path, func(c *gin.Context) { c.Status(http.StatusNoContent) })
	}
}

func (h *BenchmarkDashboardHandler) requireExperiment(c *gin.Context) {
	if _, err := h.experiments.GetExperiment(c.Request.Context(), c.Param("id")); err != nil {
		dashboardError(c, err)
		c.Abort()
		return
	}
	c.Next()
}

func dashboardError(c *gin.Context, err error) {
	switch {
	case errors.Is(err, benchmark.ErrExperimentNotFound):
		c.JSON(http.StatusNotFound, gin.H{"error": "experiment not found"})
	case errors.Is(err, benchmark.ErrInvalidQuery), errors.Is(err, metrics.ErrInvalidQuery):
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid query parameters"})
	default:
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to retrieve experiment data"})
	}
}

func (h *BenchmarkDashboardHandler) List(c *gin.Context) {
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	query, err := experimentQuery(values)
	if err != nil {
		dashboardError(c, err)
		return
	}
	page, err := h.experiments.ListExperiments(c.Request.Context(), query)
	if err != nil {
		dashboardError(c, err)
		return
	}
	c.JSON(http.StatusOK, page)
}

func (h *BenchmarkDashboardHandler) Summary(c *gin.Context) {
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	if err := validateQuery(values); err != nil {
		dashboardError(c, err)
		return
	}
	summary, err := h.dashboard.Summary(c.Request.Context(), c.Param("id"))
	if err != nil {
		dashboardError(c, err)
		return
	}
	c.JSON(http.StatusOK, summary)
}

func (h *BenchmarkDashboardHandler) Runs(c *gin.Context) {
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	if err := validateQuery(values, "limit", "offset"); err != nil {
		dashboardError(c, err)
		return
	}
	limit, offset, err := pagination(values, 20)
	if err != nil {
		dashboardError(c, err)
		return
	}
	page, err := h.dashboard.Runs(c.Request.Context(), c.Param("id"), limit, offset)
	if err != nil {
		dashboardError(c, err)
		return
	}
	c.JSON(http.StatusOK, page)
}

func (h *BenchmarkDashboardHandler) Export(c *gin.Context) {
	values, err := benchmarkQueryValues(c)
	if err != nil {
		dashboardError(c, err)
		return
	}
	if err := validateQuery(values, "format"); err != nil {
		dashboardError(c, err)
		return
	}
	format := values.Get("format")
	if format == "" {
		format = "json"
	}
	if format != "json" && format != "csv" {
		dashboardError(c, benchmark.ErrInvalidQuery)
		return
	}
	data, err := h.dashboard.Export(c.Request.Context(), c.Param("id"))
	if err != nil {
		dashboardError(c, err)
		return
	}
	// A fixed filename avoids reflecting arbitrary route identifiers into headers.
	c.Header("Content-Disposition", `attachment; filename="experiment.`+format+`"`)
	if format == "json" {
		c.JSON(http.StatusOK, data)
		return
	}
	var buffer bytes.Buffer
	writer := csv.NewWriter(&buffer)
	if err := writeExperimentCSV(writer, data); err != nil {
		c.Header("Content-Disposition", "")
		dashboardError(c, err)
		return
	}
	c.Data(http.StatusOK, "text/csv; charset=utf-8", buffer.Bytes())
}

// Escape spreadsheet formula prefixes only for text; numeric measurements keep
// their numeric representation, including negative values.
func csvText(value string) string {
	trimmed := strings.TrimLeft(value, " \t\r\n")
	if strings.ContainsAny(value[:len(value)-len(trimmed)], "\t\r\n") || len(trimmed) > 0 && strings.ContainsAny(trimmed[:1], "=+-@") {
		return "'" + value
	}
	return value
}

func csvTime(value *time.Time) string {
	if value == nil {
		return ""
	}
	return value.UTC().Format(time.RFC3339Nano)
}

func writeExperimentCSV(writer *csv.Writer, data benchmark.Export) error {
	header := []string{"experiment_id", "use_case", "dataset", "model", "model_version", "quantization", "runtime", "environment_name", "environment_type", "cpu_cores", "memory_mb", "gpu_info", "status", "result_source", "created_at", "started_at", "completed_at", "result_latency_ms", "result_peak_memory_mb", "result_tokens_per_second", "result_success", "result_error", "run_id", "batch_sequence", "event_sequence", "timestamp_ms", "name", "value", "unit", "scope", "source", "unavailable_reason"}
	if err := writer.Write(header); err != nil {
		return err
	}
	e := data.Experiment
	base := []string{csvText(e.ID), csvText(string(e.UseCase)), csvText(e.Dataset), csvText(e.ModelConfig.Name), csvText(e.ModelConfig.Version), csvText(e.ModelConfig.Quantization), csvText(e.ModelConfig.Runtime), csvText(e.EnvironmentProfile.Name), csvText(e.EnvironmentProfile.Type), strconv.Itoa(e.EnvironmentProfile.CPUCores), strconv.Itoa(e.EnvironmentProfile.MemoryMB), csvText(e.EnvironmentProfile.GPUInfo), string(e.Status), data.Summary.ResultSource, e.CreatedAt.UTC().Format(time.RFC3339Nano), csvTime(e.StartedAt), csvTime(e.CompletedAt), "", "", "", "", ""}
	if e.Result != nil {
		base[17] = strconv.Itoa(e.Result.LatencyMS)
		base[18] = strconv.Itoa(e.Result.PeakMemoryMB)
		base[19] = strconv.FormatFloat(e.Result.TokensPerSecond, 'g', -1, 64)
		base[20] = strconv.FormatBool(e.Result.Success)
		base[21] = csvText(e.Result.ErrorMessage)
	}
	wrote := false
	for _, batch := range data.Batches {
		for _, event := range batch.Events {
			value := ""
			if event.Value != nil {
				if text, ok := event.Value.(string); ok {
					value = csvText(text)
				} else {
					encoded, err := json.Marshal(event.Value)
					if err != nil {
						return err
					}
					value = string(encoded)
				}
			}
			row := append(append([]string{}, base...), csvText(batch.RunID), strconv.FormatInt(batch.SequenceNumber, 10), strconv.FormatInt(event.Sequence, 10), strconv.FormatInt(event.TimestampMS, 10), csvText(event.Name), value, csvText(event.Unit), csvText(event.Scope), csvText(event.Source), csvText(event.UnavailableReason))
			if err := writer.Write(row); err != nil {
				return err
			}
			wrote = true
		}
	}
	if !wrote {
		row := make([]string, len(header))
		copy(row, base)
		if err := writer.Write(row); err != nil {
			return err
		}
	}
	writer.Flush()
	return writer.Error()
}
