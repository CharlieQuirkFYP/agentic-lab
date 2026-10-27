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
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metricsbridge"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/voicehistory"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/voicemetrics"
	"github.com/gin-gonic/gin"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	if err := loadEnvFile(".env"); err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	config := loadConfig()
	phemeClient, err := pheme.NewClient(config.phemeURL)
	if err != nil {
		return err
	}
	voiceCORS, err := handler.NewVoiceCORS(config.corsOrigins)
	if err != nil {
		return err
	}
	benchmarkCORS, err := handler.NewBenchmarkCORS(config.corsOrigins)
	if err != nil {
		return err
	}
	database, err := storage.Open(config.databasePath)
	if err != nil {
		return err
	}
	defer database.Close()

	router := gin.Default()
	voiceService := service.NewVoiceService(phemeClient)
	voiceHandler := handler.NewVoiceHandler(voiceService)
	voiceSessionStore := voicehistory.NewStore(database)
	voiceHandler.SetSessionStore(voiceSessionStore)
	voiceSessionsHandler := handler.NewVoiceSessionsHandler(voiceSessionStore)
	voiceSessionsGroup := router.Group("/api/v1/voice", voiceCORS)
	voiceSessionsGroup.GET("/sessions", voiceSessionsHandler.List)
	voiceHandler.RegisterRoutes(router.Group("/api/v1/voice", voiceCORS))

	incidentAnalyzer := analyzer.NewMockIncidentAnalyzer()
	incidentService := service.NewIncidentService(incidentAnalyzer)
	incidentHandler := handler.NewIncidentHandler(incidentService)

	benchmarkRepository := benchmark.NewSQLiteRepository(database)
	recoveryCtx, cancelRecovery := context.WithTimeout(ctx, 30*time.Second)
	err = benchmarkRepository.RecoverInterrupted(recoveryCtx)
	cancelRecovery()
	if err != nil {
		return err
	}
	metricsRepository := metrics.NewSQLiteRepository(database)
	metricsService := metrics.NewService(metricsRepository)
	voiceMetricsHandler := handler.NewVoiceMetricsHandler(voiceService, voicemetrics.NewStore(database), benchmarkRepository, metricsService)
	voiceMetricsGroup := router.Group("/api/v1/voice", voiceCORS)
	voiceMetricsGroup.GET("/metrics", voiceMetricsHandler.Get)
	go func() {
		ticker := time.NewTicker(2 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				if err := voiceMetricsHandler.Collect(ctx); err != nil {
					log.Printf("voice metrics collection deferred: %v", err)
				}
			}
		}
	}()

	benchmarkQueue, err := benchmark.NewJobQueue(100)
	if err != nil {
		return err
	}
	benchmarkRunner := benchmark.NewPhemeRunner(phemeClient)
	benchmarkService := benchmark.NewService(benchmarkRepository, benchmarkQueue, benchmarkRunner)
	benchmarkService.SetMetricsCollector(metricsbridge.NewCollector(phemeClient, metricsService))
	benchmarkWorker := benchmark.NewWorker(benchmarkService, benchmarkQueue, 1)
	benchmarkWorker.Start(ctx)
	dashboardHandler := handler.NewBenchmarkDashboardHandler(benchmarkService, metricsService)

	router.GET("/health", handler.Health)
	router.GET("/api/v1/dashboard/events", handler.DashboardEvents)
	router.POST("/api/v1/incidents/analyze", incidentHandler.Analyze)
	dashboardHandler.RegisterRoutes(router.Group("/api/v1/experiments", benchmarkCORS), metricsService)

	server := &http.Server{
		Addr:              config.address,
		Handler:           router,
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       60 * time.Second,
		// No total read/write timeout: voice responses wait for human review.
		BaseContext: func(net.Listener) context.Context { return ctx },
	}

	shutdownDone := make(chan struct{})
	go func() {
		defer close(shutdownDone)
		<-ctx.Done()

		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		if err := server.Shutdown(shutdownCtx); err != nil {
			log.Printf("server shutdown failed: %v", err)
		}
	}()

	log.Printf("API listening on %s (API_ADDR); browser origins use API_CORS_ORIGINS; experiments stored in %s (API_DATABASE_PATH)", config.address, config.databasePath)
	listenErr := server.ListenAndServe()
	stop()
	<-shutdownDone
	benchmarkWorker.Wait()
	if !errors.Is(listenErr, http.ErrServerClosed) {
		return listenErr
	}
	return nil
}
