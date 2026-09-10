package metrics

import (
	"context"
	"fmt"
	"sync"
	"testing"
)

func TestMemoryRepositoryStoresIndependentCopiesAndRetrievesByExperiment(t *testing.T) {
	repository := NewMemoryRepository()
	batch := validBatch()
	if err := repository.AppendBatch(context.Background(), batch); err != nil {
		t.Fatalf("AppendBatch returned error: %v", err)
	}

	batch.Events[0].Name = "changed after append"
	stored, err := repository.GetByExperimentID(context.Background(), "exp-1")
	if err != nil {
		t.Fatalf("GetByExperimentID returned error: %v", err)
	}
	if len(stored) != 1 {
		t.Fatalf("batch count = %d, want 1", len(stored))
	}
	if stored[0].Events[0].Name != "latency" {
		t.Fatalf("stored event name = %q, want latency", stored[0].Events[0].Name)
	}

	stored[0].Events[0].Name = "changed after retrieval"
	again, err := repository.GetBatches(context.Background(), "run-1")
	if err != nil {
		t.Fatalf("GetBatches returned error: %v", err)
	}
	if again[0].Events[0].Name != "latency" {
		t.Fatalf("repository was not protected from caller mutation: %q", again[0].Events[0].Name)
	}
}

func TestMemoryRepositoryConcurrentAppends(t *testing.T) {
	repository := NewMemoryRepository()
	const appendCount = 100

	errorsCh := make(chan error, appendCount)
	var waitGroup sync.WaitGroup
	for index := 0; index < appendCount; index++ {
		waitGroup.Add(1)
		go func(index int) {
			defer waitGroup.Done()

			batch := validBatch()
			batch.RunID = "run-concurrent"
			batch.SequenceNumber = int64(index)
			batch.Events[0].RunID = batch.RunID
			batch.Events[0].Sequence = int64(index)
			batch.Events[0].Name = fmt.Sprintf("metric-%d", index)
			if err := repository.AppendBatch(context.Background(), batch); err != nil {
				errorsCh <- err
			}
		}(index)
	}
	waitGroup.Wait()
	close(errorsCh)

	for err := range errorsCh {
		t.Errorf("concurrent AppendBatch returned error: %v", err)
	}

	batches, err := repository.GetBatches(context.Background(), "run-concurrent")
	if err != nil {
		t.Fatalf("GetBatches returned error: %v", err)
	}
	if len(batches) != appendCount {
		t.Fatalf("batch count = %d, want %d", len(batches), appendCount)
	}

	events, err := repository.GetEvents(context.Background(), "run-concurrent")
	if err != nil {
		t.Fatalf("GetEvents returned error: %v", err)
	}
	if len(events) != appendCount {
		t.Fatalf("event count = %d, want %d", len(events), appendCount)
	}
}

func TestMemoryRepositoryHonorsCanceledContext(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	repository := NewMemoryRepository()
	if err := repository.AppendBatch(ctx, validBatch()); err != context.Canceled {
		t.Fatalf("AppendBatch error = %v, want %v", err, context.Canceled)
	}
	if _, err := repository.GetBatches(ctx, "run-1"); err != context.Canceled {
		t.Fatalf("GetBatches error = %v, want %v", err, context.Canceled)
	}
	if _, err := repository.GetByExperimentID(ctx, "exp-1"); err != context.Canceled {
		t.Fatalf("GetByExperimentID error = %v, want %v", err, context.Canceled)
	}
}

func TestMemoryRepositoryGetBatch(t *testing.T) {
	repository := NewMemoryRepository()
	batch := validBatch()
	batch.SequenceNumber = 4
	if err := repository.AppendBatch(context.Background(), batch); err != nil {
		t.Fatalf("AppendBatch returned error: %v", err)
	}

	stored, err := repository.GetBatch(context.Background(), batch.RunID, batch.SequenceNumber)
	if err != nil {
		t.Fatalf("GetBatch returned error: %v", err)
	}
	if stored.SequenceNumber != batch.SequenceNumber {
		t.Fatalf("sequence number = %d, want %d", stored.SequenceNumber, batch.SequenceNumber)
	}
	if _, err := repository.GetBatch(context.Background(), batch.RunID, 99); err != ErrMetricsBatchNotFound {
		t.Fatalf("missing GetBatch error = %v, want %v", err, ErrMetricsBatchNotFound)
	}
}
