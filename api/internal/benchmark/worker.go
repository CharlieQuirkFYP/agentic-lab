package benchmark

import (
	"context"
	"log"
	"sync"
)

type Worker struct {
	service     *Service
	queue       *JobQueue
	workerCount int
	wg          sync.WaitGroup
}

func NewWorker(service *Service, queue *JobQueue, workerCount int) *Worker {
	if workerCount < 1 {
		workerCount = 1
	}

	return &Worker{
		service:     service,
		queue:       queue,
		workerCount: workerCount,
	}
}

func (w *Worker) Start(ctx context.Context) {
	for i := 0; i < w.workerCount; i++ {
		w.wg.Add(1)
		go w.run(ctx)
	}
}

// Wait joins workers after their context is canceled, before storage is closed.
func (w *Worker) Wait() { w.wg.Wait() }

func (w *Worker) run(ctx context.Context) {
	defer w.wg.Done()
	for {
		select {
		case <-ctx.Done():
			return
		case job := <-w.queue.Jobs():
			if ctx.Err() != nil {
				return
			}
			if err := w.service.ProcessExperiment(ctx, job.ExperimentID); err != nil {
				log.Printf("benchmark worker failed to process experiment %s: %v", job.ExperimentID, err)
			}
		}
	}
}
