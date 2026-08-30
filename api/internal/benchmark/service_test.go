package benchmark

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

type staticRunner struct {
	result Result
	err    error
}

func (r staticRunner) Run(ctx context.Context, experiment Experiment) (Result, error) {
	return r.result, r.err
}

type blockingRunner struct {
	started chan Experiment
	release chan struct{}
	result  Result
	err     error
}

func (r *blockingRunner) Run(ctx context.Context, experiment Experiment) (Result, error) {
	r.started <- experiment

	select {
	case <-ctx.Done():
		return Result{Success: false, ErrorMessage: ctx.Err().Error()}, ctx.Err()
	case <-r.release:
		return r.result, r.err
	}
}

type failingQueue struct {
	err          error
	beforeReturn func()
}

func (q failingQueue) Enqueue(ctx context.Context, job Job) error {
	if q.beforeReturn != nil {
		q.beforeReturn()
	}

	return q.err
}

func TestCreateExperimentStoresQueuedExperiment(t *testing.T) {
	ctx := context.Background()
	repository := NewMemoryRepository()
	queue, err := NewJobQueue(1)
	if err != nil {
		t.Fatalf("NewJobQueue returned error: %v", err)
	}
	service := NewService(repository, queue, staticRunner{
		result: Result{Success: true},
	})

	experiment, err := service.CreateExperiment(ctx, CreateExperimentRequest{
		UseCase: UseCaseIncidentReporting,
		Dataset: "sample-dataset",
		ModelConfig: ModelConfig{
			Name: "mock-model",
		},
		EnvironmentProfile: EnvironmentProfile{
			Name:     "local",
			Type:     "developer-machine",
			CPUCores: 4,
			MemoryMB: 8192,
		},
	})
	if err != nil {
		t.Fatalf("CreateExperiment returned error: %v", err)
	}

	if experiment.ID == "" {
		t.Fatal("CreateExperiment returned empty ID")
	}
	if experiment.Status != ExperimentStatusQueued {
		t.Fatalf("CreateExperiment status = %q, want %q", experiment.Status, ExperimentStatusQueued)
	}

	stored, err := repository.GetByID(ctx, experiment.ID)
	if err != nil {
		t.Fatalf("GetByID returned error: %v", err)
	}
	if stored.Status != ExperimentStatusQueued {
		t.Fatalf("stored status = %q, want %q", stored.Status, ExperimentStatusQueued)
	}
}

func TestCreateExperimentMarksExperimentFailedWhenEnqueueFails(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	repository := NewMemoryRepository()
	enqueueErr := errors.New("enqueue unavailable")
	service := NewService(repository, failingQueue{
		err:          enqueueErr,
		beforeReturn: cancel,
	}, staticRunner{})

	_, err := service.CreateExperiment(ctx, CreateExperimentRequest{
		UseCase: UseCaseIncidentReporting,
	})
	if !errors.Is(err, enqueueErr) {
		t.Fatalf("CreateExperiment error = %v, want %v", err, enqueueErr)
	}

	stored := onlyStoredExperiment(t, repository)
	if stored.ID == "" {
		t.Fatal("stored experiment ID is empty")
	}
	if stored.Status != ExperimentStatusFailed {
		t.Fatalf("stored status = %q, want %q", stored.Status, ExperimentStatusFailed)
	}
	if stored.CompletedAt == nil {
		t.Fatal("CompletedAt was not set")
	}
	if stored.Result == nil {
		t.Fatal("Result was not set")
	}
	if stored.Result.Success {
		t.Fatal("Result.Success = true, want false")
	}
	if !strings.Contains(stored.Result.ErrorMessage, enqueueErr.Error()) {
		t.Fatalf("Result.ErrorMessage = %q, want it to contain %q", stored.Result.ErrorMessage, enqueueErr.Error())
	}
}

func TestWorkerTransitionsExperimentToRunningAndCompleted(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	repository := NewMemoryRepository()
	queue, err := NewJobQueue(1)
	if err != nil {
		t.Fatalf("NewJobQueue returned error: %v", err)
	}
	runner := &blockingRunner{
		started: make(chan Experiment, 1),
		release: make(chan struct{}),
		result: Result{
			LatencyMS:       99,
			PeakMemoryMB:    128,
			TokensPerSecond: 12.5,
			Success:         true,
		},
	}
	service := NewService(repository, queue, runner)
	worker := NewWorker(service, queue, 1)
	worker.Start(ctx)

	experiment := Experiment{
		ID:        "exp_worker_success",
		UseCase:   UseCaseIncidentReporting,
		Status:    ExperimentStatusQueued,
		CreatedAt: time.Now().UTC(),
	}
	if err := repository.Create(ctx, experiment); err != nil {
		t.Fatalf("Create returned error: %v", err)
	}
	if err := queue.Enqueue(ctx, Job{ExperimentID: experiment.ID}); err != nil {
		t.Fatalf("Enqueue returned error: %v", err)
	}

	select {
	case <-runner.started:
	case <-time.After(time.Second):
		t.Fatal("runner was not called")
	}

	running, err := repository.GetByID(ctx, experiment.ID)
	if err != nil {
		t.Fatalf("GetByID returned error: %v", err)
	}
	if running.Status != ExperimentStatusRunning {
		t.Fatalf("status = %q, want %q", running.Status, ExperimentStatusRunning)
	}
	if running.StartedAt == nil {
		t.Fatal("StartedAt was not set")
	}

	close(runner.release)

	completed := waitForExperimentStatus(t, repository, experiment.ID, ExperimentStatusCompleted)
	if completed.CompletedAt == nil {
		t.Fatal("CompletedAt was not set")
	}
	if completed.Result == nil {
		t.Fatal("result was not stored")
	}
	if *completed.Result != runner.result {
		t.Fatalf("result = %+v, want %+v", *completed.Result, runner.result)
	}
}

func TestProcessExperimentTransitionsFailedRunnerToFailed(t *testing.T) {
	ctx := context.Background()
	repository := NewMemoryRepository()
	queue, err := NewJobQueue(1)
	if err != nil {
		t.Fatalf("NewJobQueue returned error: %v", err)
	}
	runErr := errors.New("mock runner failed")
	service := NewService(repository, queue, staticRunner{
		result: Result{
			LatencyMS:    10,
			PeakMemoryMB: 64,
		},
		err: runErr,
	})

	experiment := Experiment{
		ID:        "exp_worker_failure",
		UseCase:   UseCaseInterviewAssistant,
		Status:    ExperimentStatusQueued,
		CreatedAt: time.Now().UTC(),
	}
	if err := repository.Create(ctx, experiment); err != nil {
		t.Fatalf("Create returned error: %v", err)
	}

	err = service.ProcessExperiment(ctx, experiment.ID)
	if !errors.Is(err, runErr) {
		t.Fatalf("ProcessExperiment error = %v, want %v", err, runErr)
	}

	failed, err := repository.GetByID(ctx, experiment.ID)
	if err != nil {
		t.Fatalf("GetByID returned error: %v", err)
	}
	if failed.Status != ExperimentStatusFailed {
		t.Fatalf("status = %q, want %q", failed.Status, ExperimentStatusFailed)
	}
	if failed.Result == nil {
		t.Fatal("failure result was not stored")
	}
	if failed.Result.Success {
		t.Fatal("failure result Success = true, want false")
	}
	if failed.Result.ErrorMessage != runErr.Error() {
		t.Fatalf("failure error = %q, want %q", failed.Result.ErrorMessage, runErr.Error())
	}
	if failed.CompletedAt == nil {
		t.Fatal("CompletedAt was not set")
	}
}

func TestMemoryRepositoryConcurrentAccess(t *testing.T) {
	ctx := context.Background()
	repository := NewMemoryRepository()

	var wg sync.WaitGroup
	for i := 0; i < 100; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()

			id := fmt.Sprintf("exp_%03d", i)
			experiment := Experiment{
				ID:        id,
				UseCase:   UseCaseLicensePlateMonitoring,
				Status:    ExperimentStatusQueued,
				CreatedAt: time.Now().UTC(),
			}
			if err := repository.Create(ctx, experiment); err != nil {
				t.Errorf("Create returned error: %v", err)
				return
			}

			experiment.Status = ExperimentStatusRunning
			if err := repository.Update(ctx, experiment); err != nil {
				t.Errorf("Update returned error: %v", err)
				return
			}

			stored, err := repository.GetByID(ctx, id)
			if err != nil {
				t.Errorf("GetByID returned error: %v", err)
				return
			}
			if stored.Status != ExperimentStatusRunning {
				t.Errorf("status = %q, want %q", stored.Status, ExperimentStatusRunning)
			}
		}(i)
	}

	wg.Wait()
}

func onlyStoredExperiment(t *testing.T, repository *MemoryRepository) Experiment {
	t.Helper()

	repository.mu.RLock()
	defer repository.mu.RUnlock()

	if len(repository.experiments) != 1 {
		t.Fatalf("stored experiment count = %d, want 1", len(repository.experiments))
	}
	for _, experiment := range repository.experiments {
		return cloneExperiment(experiment)
	}

	t.Fatal("expected stored experiment")
	return Experiment{}
}

func waitForExperimentStatus(t *testing.T, repository Repository, id string, status ExperimentStatus) Experiment {
	t.Helper()

	ctx := context.Background()
	deadline := time.After(time.Second)
	ticker := time.NewTicker(5 * time.Millisecond)
	defer ticker.Stop()

	for {
		select {
		case <-deadline:
			experiment, err := repository.GetByID(ctx, id)
			if err != nil {
				t.Fatalf("GetByID returned error: %v", err)
			}
			t.Fatalf("timed out waiting for status %q, got %q", status, experiment.Status)
		case <-ticker.C:
			experiment, err := repository.GetByID(ctx, id)
			if err != nil {
				t.Fatalf("GetByID returned error: %v", err)
			}
			if experiment.Status == status {
				return experiment
			}
		}
	}
}
