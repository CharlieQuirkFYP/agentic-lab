package benchmark

import (
	"context"
	"errors"
)

var ErrInvalidQueueCapacity = errors.New("queue capacity must be greater than zero")

type Job struct {
	ExperimentID string
}

type Queue interface {
	Enqueue(ctx context.Context, job Job) error
}

type JobQueue struct {
	jobs chan Job
}

func NewJobQueue(capacity int) (*JobQueue, error) {
	if capacity <= 0 {
		return nil, ErrInvalidQueueCapacity
	}

	return &JobQueue{
		jobs: make(chan Job, capacity),
	}, nil
}

func (q *JobQueue) Enqueue(ctx context.Context, job Job) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case q.jobs <- job:
		return nil
	}
}

func (q *JobQueue) Jobs() <-chan Job {
	return q.jobs
}
