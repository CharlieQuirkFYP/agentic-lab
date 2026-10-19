package benchmark

import (
	"context"
	"errors"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
)

func TestRepositoryHistoryFiltersAndPagination(t *testing.T) {
	db, err := storage.Open(filepath.Join(t.TempDir(), "experiments.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for name, repository := range map[string]Repository{"memory": NewMemoryRepository(), "sqlite": NewSQLiteRepository(db)} {
		t.Run(name, func(t *testing.T) {
			ctx := t.Context()
			base := time.Date(2026, 10, 19, 1, 0, 0, 0, time.UTC)
			for i, id := range []string{"a", "b", "c", "d"} {
				e := Experiment{ID: id, Status: ExperimentStatusCompleted, UseCase: UseCaseIncidentReporting, Dataset: "dataset-1", ModelConfig: ModelConfig{Name: "model-1"}, CreatedAt: base.Add(time.Duration(i/2) * time.Second)}
				if id == "d" {
					e.Status = ExperimentStatusFailed
					e.Dataset = "dataset-2"
					e.ModelConfig.Name = "model-2"
					e.UseCase = UseCaseInterviewAssistant
				}
				if err := repository.Create(ctx, e); err != nil {
					t.Fatal(err)
				}
			}
			bound := base.Add(time.Second)
			for _, tc := range []struct {
				name  string
				q     ListQuery
				ids   []string
				total int
			}{
				{"stable order", ListQuery{Limit: 2}, []string{"d", "c"}, 4},
				{"second page", ListQuery{Limit: 2, Offset: 2}, []string{"b", "a"}, 4},
				{"empty page", ListQuery{Limit: 2, Offset: 999}, []string{}, 4},
				{"status", ListQuery{Limit: 20, Status: ExperimentStatusFailed}, []string{"d"}, 1},
				{"model", ListQuery{Limit: 20, Model: "model-1"}, []string{"c", "b", "a"}, 3},
				{"dataset", ListQuery{Limit: 20, Dataset: "dataset-2"}, []string{"d"}, 1},
				{"use case", ListQuery{Limit: 20, UseCase: UseCaseInterviewAssistant}, []string{"d"}, 1},
				{"inclusive bounds", ListQuery{Limit: 20, CreatedAfter: &bound, CreatedBefore: &bound}, []string{"d", "c"}, 2},
				{"combined filters", ListQuery{Limit: 20, Status: ExperimentStatusCompleted, Model: "model-1", CreatedAfter: &bound}, []string{"c"}, 1},
				{"literal SQL characters", ListQuery{Limit: 20, Model: "' OR 1=1 --"}, []string{}, 0},
			} {
				t.Run(tc.name, func(t *testing.T) {
					page, err := repository.List(ctx, tc.q)
					if err != nil {
						t.Fatal(err)
					}
					ids := []string{}
					for _, e := range page.Experiments {
						ids = append(ids, e.ID)
					}
					if !reflect.DeepEqual(ids, tc.ids) || page.Total != tc.total {
						t.Fatalf("page=%+v ids=%v want=%v total=%d", page, ids, tc.ids, tc.total)
					}
				})
			}
			canceled, cancel := context.WithCancel(ctx)
			cancel()
			if _, err := repository.List(canceled, ListQuery{Limit: 20}); !errors.Is(err, context.Canceled) {
				t.Fatalf("cancellation: %v", err)
			}
		})
	}
}

func TestSQLiteRestartPreservesCompletedAndFailsInterrupted(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "experiments.sqlite")
	db, err := storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	r := NewSQLiteRepository(db)
	ctx := t.Context()
	queue, err := NewJobQueue(1)
	if err != nil {
		t.Fatal(err)
	}
	service := NewService(r, queue, NewMockRunner())
	e, err := service.CreateExperiment(ctx, CreateExperimentRequest{UseCase: UseCaseIncidentReporting, Dataset: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if err := service.ProcessExperiment(ctx, e.ID); err != nil {
		t.Fatal(err)
	}
	before, err := r.GetByID(ctx, e.ID)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 105; i++ { // Exercise recovery across more than one history page.
		status := ExperimentStatusQueued
		if i%2 == 0 {
			status = ExperimentStatusRunning
		}
		if err := r.Create(ctx, Experiment{ID: newExperimentID(), Status: status, CreatedAt: time.Now().UTC()}); err != nil {
			t.Fatal(err)
		}
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	db, err = storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	r = NewSQLiteRepository(db)
	if err := r.RecoverInterrupted(ctx); err != nil {
		t.Fatal(err)
	}
	if err := r.RecoverInterrupted(ctx); err != nil {
		t.Fatal(err)
	}
	after, err := r.GetByID(ctx, e.ID)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(before, after) || after.ResultSource != "mock" {
		t.Fatalf("completed result changed across restart: %+v", after)
	}
	failed, err := r.List(ctx, ListQuery{Limit: 100, Status: ExperimentStatusFailed})
	if err != nil {
		t.Fatal(err)
	}
	if failed.Total != 105 {
		t.Fatalf("failed total=%d", failed.Total)
	}
	for _, e := range failed.Experiments {
		if e.CompletedAt == nil || e.Result == nil || e.Result.Success || e.Result.ErrorMessage == "" {
			t.Fatalf("missing interruption details: %+v", e)
		}
	}
	if _, err := r.GetByID(ctx, "missing"); !errors.Is(err, ErrExperimentNotFound) {
		t.Fatalf("missing get: %v", err)
	}
	if err := r.Update(ctx, Experiment{ID: "missing"}); !errors.Is(err, ErrExperimentNotFound) {
		t.Fatalf("missing update: %v", err)
	}
}
