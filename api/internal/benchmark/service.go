package benchmark

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"log"
	"time"
)

var (
	ErrUseCaseRequired    = errors.New("use_case is required")
	ErrUnsupportedUseCase = errors.New("unsupported use_case")
)

type Service struct {
	repository Repository
	queue      Queue
	runner     Runner
	collector  MetricsCollector
	notify     func()
}

// MetricsCollector imports measurements produced during a benchmark run.
// It is optional so isolated benchmark tests and the mock runner remain
// independent from a metrics producer.
type MetricsCollector interface {
	Collect(context.Context, Experiment) error
}

func NewService(repository Repository, queue Queue, runner Runner) *Service {
	return &Service{
		repository: repository,
		queue:      queue,
		runner:     runner,
	}
}

func (s *Service) SetMetricsCollector(collector MetricsCollector) {
	s.collector = collector
}

func (s *Service) SetUpdateNotifier(notify func()) { s.notify = notify }

func (s *Service) CreateExperiment(ctx context.Context, req CreateExperimentRequest) (Experiment, error) {
	if req.UseCase == "" {
		return Experiment{}, ErrUseCaseRequired
	}
	if !req.UseCase.IsSupported() {
		return Experiment{}, ErrUnsupportedUseCase
	}

	experiment := Experiment{
		ResultSource:       "unavailable",
		ID:                 newExperimentID(),
		UseCase:            req.UseCase,
		Dataset:            req.Dataset,
		Cases:              append([]Case(nil), req.Cases...),
		ModelConfig:        req.ModelConfig,
		EnvironmentProfile: req.EnvironmentProfile,
		Status:             ExperimentStatusQueued,
		CreatedAt:          time.Now().UTC(),
	}
	if source, ok := s.runner.(interface{ ResultSource() string }); ok {
		experiment.ResultSource = source.ResultSource()
	}

	if err := s.repository.Create(ctx, experiment); err != nil {
		return Experiment{}, err
	}
	if err := s.queue.Enqueue(ctx, Job{ExperimentID: experiment.ID}); err != nil {
		s.markEnqueueFailed(experiment, err)
		return Experiment{}, err
	}
	if s.notify != nil {
		s.notify()
	}

	return experiment, nil
}

func (s *Service) GetExperiment(ctx context.Context, id string) (Experiment, error) {
	return s.repository.GetByID(ctx, id)
}

func (s *Service) ProcessExperiment(ctx context.Context, id string) error {
	experiment, err := s.repository.GetByID(ctx, id)
	if err != nil {
		return err
	}

	startedAt := time.Now().UTC()
	experiment.Status = ExperimentStatusRunning
	experiment.StartedAt = &startedAt
	experiment.Result = nil

	if err := s.repository.Update(ctx, experiment); err != nil {
		return err
	}

	result, runErr := s.runner.Run(ctx, experiment)
	if s.collector != nil {
		if err := s.collector.Collect(ctx, experiment); err != nil {
			log.Printf("failed to import metrics for experiment %s: %v", experiment.ID, err)
		}
	}
	completedAt := time.Now().UTC()
	experiment.CompletedAt = &completedAt
	experiment.Result = &result

	if runErr != nil {
		experiment.Status = ExperimentStatusFailed
		if experiment.Result.ErrorMessage == "" {
			experiment.Result.ErrorMessage = runErr.Error()
		}
		experiment.Result.Success = false
	} else {
		experiment.Status = ExperimentStatusCompleted
	}

	if err := s.repository.Update(context.Background(), experiment); err != nil {
		return err
	}
	if s.notify != nil {
		s.notify()
	}

	return runErr
}

func newExperimentID() string {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err == nil {
		return "exp_" + hex.EncodeToString(bytes[:])
	}

	return fmt.Sprintf("exp_%d", time.Now().UTC().UnixNano())
}

func (s *Service) markEnqueueFailed(experiment Experiment, enqueueErr error) {
	completedAt := time.Now().UTC()
	experiment.Status = ExperimentStatusFailed
	experiment.CompletedAt = &completedAt
	experiment.Result = &Result{
		Success:      false,
		ErrorMessage: enqueueErr.Error(),
	}

	cleanupCtx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	if err := s.repository.Update(cleanupCtx, experiment); err != nil {
		log.Printf("failed to persist enqueue failure for experiment %s: %v", experiment.ID, err)
	}
}
