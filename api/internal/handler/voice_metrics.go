package handler

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/voicemetrics"
	"github.com/gin-gonic/gin"
)

type VoiceMetricsHandler struct {
	voice          *service.VoiceService
	store          *voicemetrics.Store
	experiments    benchmark.Repository
	benchmarkStore *metrics.Service
	notify         func()
}

func NewVoiceMetricsHandler(voice *service.VoiceService, store *voicemetrics.Store, experiments benchmark.Repository, benchmarkStore *metrics.Service) *VoiceMetricsHandler {
	return &VoiceMetricsHandler{voice: voice, store: store, experiments: experiments, benchmarkStore: benchmarkStore}
}

func (h *VoiceMetricsHandler) SetUpdateNotifier(notify func()) { h.notify = notify }

// Collect drains Pheme's in-memory batcher and persists the batches in the
// Go-owned SQLite database. It is safe to call periodically and is idempotent
// at the database layer for already-seen batch identities.
func (h *VoiceMetricsHandler) Collect(ctx context.Context) error {
	batches, err := h.voice.DrainMetrics(ctx)
	if err != nil {
		return err
	}
	for _, batch := range batches {
		if err := h.store.Append(ctx, batch); err != nil {
			return err
		}
		experimentID := phoneExperimentID(batch.RunID)
		if _, err := h.experiments.GetByID(ctx, experimentID); err != nil {
			if err != benchmark.ErrExperimentNotFound {
				return err
			}
			if err := h.experiments.Create(ctx, benchmark.Experiment{
				ID: experimentID, ResultSource: "phone_voice", UseCase: benchmark.UseCaseIncidentReporting,
				Dataset: "phone", ModelConfig: benchmark.ModelConfig{Name: "Pheme voice"},
				Status: benchmark.ExperimentStatusCompleted, CreatedAt: time.Now().UTC(),
			}); err != nil {
				return err
			}
		}
		batch.ExperimentID = experimentID
		for index := range batch.Events {
			batch.Events[index].ExperimentID = experimentID
		}
		if err := h.benchmarkStore.Append(ctx, batch); err != nil {
			return err
		}
	}
	if len(batches) > 0 && h.notify != nil {
		h.notify()
	}
	return nil
}

func phoneExperimentID(runID string) string {
	digest := sha256.Sum256([]byte(runID))
	return "phone_" + hex.EncodeToString(digest[:12])
}

func (h *VoiceMetricsHandler) Get(c *gin.Context) {
	stored, err := h.store.List(c.Request.Context())
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "could not read voice metrics"})
		return
	}
	c.JSON(http.StatusOK, gin.H{"source": "phone_voice", "batches": stored, "total": len(stored)})
}
