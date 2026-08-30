package benchmark

import (
	"context"
	"log"
)

type Worker struct {
	service     *Service
	queue       *JobQueue
	workerCount int
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
		go w.run(ctx)
	}
}

func (w *Worker) run(ctx context.Context) {
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
