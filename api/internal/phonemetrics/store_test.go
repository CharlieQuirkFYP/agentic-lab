package phonemetrics

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"sync"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
)

func fixture() Upload {
	return Upload{SchemaVersion: 1,
		Device: Device{InstallationID: "install-1", Hardware: "iPhone-test", OSVersion: "27.0", AppVersion: "0.1", Build: "1"},
		Batch: metrics.Batch{SchemaVersion: 1, RunID: "native-run-1", SequenceNumber: 0, TimestampMS: 1000,
			Events: []metrics.Event{
				{SchemaVersion: 1, RunID: "native-run-1", Sequence: 0, TimestampMS: 1000, Name: "transcription_duration_ms", Value: 42.5, Unit: "milliseconds", Scope: "run", Source: "core.transcription"},
				{SchemaVersion: 1, RunID: "native-run-1", Sequence: 1, TimestampMS: 1000, Name: "whole_device_power_watts", Value: nil, Unit: "watts", Scope: "device", Source: "native.ios", UnavailableReason: "no verified power meter"},
			}},
	}
}

func TestPersistRetrySummaryAndRestart(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "metrics.sqlite")
	db, err := storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	store := NewStore(db)
	upload := fixture()
	id, err := store.Append(ctx, upload)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.Append(ctx, upload); err != nil {
		t.Fatal(err)
	}
	page, err := store.List(ctx, 100, 0)
	if err != nil || page.Total != 1 || page.Source != Source {
		t.Fatalf("page = %+v, err = %v", page, err)
	}
	repository := benchmark.NewSQLiteRepository(db)
	experiment, err := repository.GetByID(ctx, id)
	if err != nil || experiment.ResultSource != Source || experiment.Result != nil ||
		experiment.Status != benchmark.ExperimentStatusCompleted || experiment.Device.ExecutionLocation != "device" {
		t.Fatalf("experiment = %+v, err = %v", experiment, err)
	}
	dashboard := benchmark.NewDashboardService(benchmark.NewService(repository, nil, nil), metrics.NewService(metrics.NewSQLiteRepository(db)))
	summary, err := dashboard.Summary(ctx, id)
	if err != nil || summary.RunCount != 1 || len(summary.Metrics) != 2 {
		t.Fatalf("summary = %+v, err = %v", summary, err)
	}
	for _, metric := range summary.Metrics {
		if metric.Name == "transcription_duration_ms" && (metric.Mean == nil || *metric.Mean != 42.5) {
			t.Fatalf("wrong timing: %+v", metric)
		}
		if metric.Name == "whole_device_power_watts" && (metric.Mean != nil || metric.UnavailableCount != 1) {
			t.Fatalf("unavailable power became a number: %+v", metric)
		}
	}
	changed := fixture()
	changed.Batch.Events[0].Value = 99
	if _, err := store.Append(ctx, changed); !errors.Is(err, metrics.ErrBatchConflict) {
		t.Fatalf("changed retry: %v", err)
	}
	changed = fixture()
	changed.Device.Build = "2"
	if _, err := store.Append(ctx, changed); !errors.Is(err, metrics.ErrBatchConflict) {
		t.Fatalf("changed provenance: %v", err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	db, err = storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	page, err = NewStore(db).List(ctx, 100, 0)
	if err != nil || page.Total != 1 {
		t.Fatalf("restart page = %+v, err = %v", page, err)
	}
}

func TestConcurrentRetriesAndInstallationIsolation(t *testing.T) {
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	store := NewStore(db)
	var wait sync.WaitGroup
	for range 8 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			if _, err := store.Append(context.Background(), fixture()); err != nil {
				t.Errorf("concurrent append: %v", err)
			}
		}()
	}
	wait.Wait()
	other := fixture()
	other.Device.InstallationID = "install-2"
	if _, err := store.Append(context.Background(), other); err != nil {
		t.Fatal(err)
	}
	page, err := store.List(context.Background(), 1, 1)
	if err != nil || page.Total != 2 || len(page.Batches) != 1 {
		t.Fatalf("page = %+v, err = %v", page, err)
	}
}

func TestInvalidUploadsDoNotRegisterExperiments(t *testing.T) {
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	store := NewStore(db)
	for _, change := range []func(*Upload){
		func(u *Upload) { u.SchemaVersion = 2 },
		func(u *Upload) { u.Device.InstallationID = "" },
		func(u *Upload) { u.Device.InstallationID = "install\x00other" },
		func(u *Upload) {
			u.Batch.RunID = "run\x00other"
			for i := range u.Batch.Events {
				u.Batch.Events[i].RunID = u.Batch.RunID
			}
		},
		func(u *Upload) { u.Batch.ExperimentID = "exp-desktop" },
		func(u *Upload) { u.Batch.Events[0].ExperimentID = "exp-desktop" },
		func(u *Upload) { u.Batch.Events[1].UnavailableReason = "" },
		func(u *Upload) { u.Batch.Events = nil },
	} {
		upload := fixture()
		change(&upload)
		if _, err := store.Append(context.Background(), upload); !errors.Is(err, ErrInvalidUpload) {
			t.Fatalf("invalid upload accepted: %v", err)
		}
	}
	raw, _ := json.Marshal(fixture())
	var fields map[string]json.RawMessage
	_ = json.Unmarshal(raw, &fields)
	fields["batch"] = json.RawMessage(`{"schema_version":1,"run_id":"run-1","timestamp_ms":1,"events":[]}`)
	raw, _ = json.Marshal(fields)
	var upload Upload
	_ = json.Unmarshal(raw, &upload)
	if _, err := store.Append(context.Background(), upload); !errors.Is(err, ErrInvalidUpload) {
		t.Fatal("accepted missing sequence")
	}
	var count int
	_ = db.QueryRow("SELECT COUNT(*) FROM experiments").Scan(&count)
	if count != 0 {
		t.Fatal("invalid uploads created experiments")
	}
}
