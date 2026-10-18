package main

import (
	"context"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/analyzer"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/handler"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/gin-gonic/gin"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	config := loadConfig()
	phemeClient, err := pheme.NewClient(config.phemeURL)
	if err != nil {
		log.Fatal(err)
	}
	voiceCORS, err := handler.NewVoiceCORS(config.corsOrigins)
	if err != nil {
		log.Fatal(err)
	}

	router := gin.Default()
	voiceService := service.NewVoiceService(phemeClient)
	voiceHandler := handler.NewVoiceHandler(voiceService)
	voiceHandler.RegisterRoutes(router.Group("/api/v1/voice", voiceCORS))

	incidentAnalyzer := analyzer.NewMockIncidentAnalyzer()
	incidentService := service.NewIncidentService(incidentAnalyzer)
	incidentHandler := handler.NewIncidentHandler(incidentService)

	benchmarkRepository := benchmark.NewMemoryRepository()
	metricsRepository := metrics.NewMemoryRepository()
	metricsService := metrics.NewService(metricsRepository)
	metricsHandler := handler.NewMetricsHandler(metricsService)

	benchmarkQueue, err := benchmark.NewJobQueue(100)
	if err != nil {
		log.Fatal(err)
	}
	benchmarkRunner := benchmark.NewMockRunner()
	benchmarkService := benchmark.NewService(benchmarkRepository, benchmarkQueue, benchmarkRunner)
	benchmarkWorker := benchmark.NewWorker(benchmarkService, benchmarkQueue, 1)
	benchmarkWorker.Start(ctx)
	experimentHandler := handler.NewExperimentHandler(benchmarkService)

	router.GET("/health", handler.Health)
	router.POST("/api/v1/incidents/analyze", incidentHandler.Analyze)
	router.POST("/api/v1/experiments", experimentHandler.Create)
	router.GET("/api/v1/experiments/:id", experimentHandler.Get)
	router.POST("/api/v1/experiments/:id/metrics", metricsHandler.Append)
	router.GET("/api/v1/experiments/:id/metrics", metricsHandler.Get)

	server := &http.Server{
		Addr:              config.address,
		Handler:           router,
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       60 * time.Second,
		// No total read/write timeout: voice responses wait for human review.
		BaseContext: func(net.Listener) context.Context { return ctx },
	}

	go func() {
		<-ctx.Done()

		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		if err := server.Shutdown(shutdownCtx); err != nil {
			log.Printf("server shutdown failed: %v", err)
		}
	}()

	log.Printf("API listening on %s (API_ADDR); voice browser origins use API_CORS_ORIGINS", config.address)
	if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}
