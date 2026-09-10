package metrics

import (
	"context"
	"errors"
	"testing"
)

func TestServiceValidatesBeforeAppending(t *testing.T) {
	repository := &recordingRepository{}
	service := NewService(repository)

	invalid := validBatch()
	invalid.RunID = ""
	if err := service.Append(context.Background(), invalid); !errors.Is(err, ErrRunIDRequired) {
		t.Fatalf("Append error = %v, want ErrRunIDRequired", err)
	}
	if repository.appended != 0 {
		t.Fatalf("repository append count = %d, want 0", repository.appended)
	}

	if err := service.Append(context.Background(), validBatch()); err != nil {
		t.Fatalf("Append returned error: %v", err)
	}
	if repository.appended != 1 {
		t.Fatalf("repository append count = %d, want 1", repository.appended)
	}
}

func TestServicePropagatesCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	repository := &recordingRepository{}
	service := NewService(repository)
	if err := service.Append(ctx, validBatch()); err != context.Canceled {
		t.Fatalf("Append error = %v, want %v", err, context.Canceled)
	}
	if repository.appended != 0 {
		t.Fatalf("repository append count = %d, want 0", repository.appended)
	}
}

func TestServiceRetrievesByExperimentID(t *testing.T) {
	repository := NewMemoryRepository()
	service := NewService(repository)
	if err := service.Append(context.Background(), validBatch()); err != nil {
		t.Fatalf("Append returned error: %v", err)
	}

	batches, err := service.GetByExperimentID(context.Background(), "exp-1")
	if err != nil {
		t.Fatalf("GetByExperimentID returned error: %v", err)
	}
	if len(batches) != 1 || batches[0].RunID != "run-1" {
		t.Fatalf("batches = %#v, want one run-1 batch", batches)
	}
}

func TestServiceReportsUnsupportedReadSide(t *testing.T) {
	service := NewService(&recordingRepository{})
	if _, err := service.GetByExperimentID(context.Background(), "exp-1"); err != ErrRepositoryReadUnsupported {
		t.Fatalf("GetByExperimentID error = %v, want %v", err, ErrRepositoryReadUnsupported)
	}
}

type recordingRepository struct {
	appended int
}

func (r *recordingRepository) AppendBatch(ctx context.Context, batch Batch) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	r.appended++
	return nil
}
