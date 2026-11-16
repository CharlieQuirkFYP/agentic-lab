package phoneobservations

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
)

func device() phonemetrics.Device {
	return phonemetrics.Device{InstallationID: "install-a", Hardware: "fixture-phone", OSVersion: "test", AppVersion: "1", Build: "1"}
}
func observation(n int, kind, id string, revision int64, ctx Context, data string) Observation {
	var m map[string]json.RawMessage
	if err := json.Unmarshal([]byte(data), &m); err != nil {
		panic(err)
	}
	return Observation{SchemaVersion: 1, RecordID: fmt.Sprintf("record-%d", n), ProducerStreamID: "stream", RecordSequence: int64(n), TimestampMS: int64(n + 1000), Kind: kind, EntityID: id, Revision: revision, Context: ctx, Data: m}
}
func upload(rs ...Observation) Upload { return Upload{SchemaVersion: 1, Device: device(), Records: rs} }
func batch(run string, seq int64, value any) metrics.Batch {
	return metrics.Batch{SchemaVersion: 1, RunID: run, SequenceNumber: seq, TimestampMS: 1000 + seq, Events: []metrics.Event{{SchemaVersion: 1, RunID: run, Sequence: seq, TimestampMS: 1000 + seq, Name: "duration_ms", Value: value, Unit: "milliseconds", Scope: "run", Source: "fixture"}}}
}
func batchObservation(n int, attempt, run string, seq int64, value any) Observation {
	raw, _ := json.Marshal(map[string]any{"batch": batch(run, seq, value)})
	return observation(n, "metric_batch", fmt.Sprintf("batch-%d", n), 0, Context{AttemptID: attempt}, string(raw))
}
func testService(t *testing.T) (*Store, *Service) {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	s := NewStore(db)
	return s, NewService(s)
}
func appendOK(t *testing.T, s *Store, u Upload) {
	t.Helper()
	if _, err := s.Append(context.Background(), u); err != nil {
		t.Fatal(err)
	}
}
func TestAtomicIdentityConflictAndInstallationIsolation(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	initial := observation(1, "conversation", "chat", 1, Context{}, `{"status":"active","source":"manual"}`)
	appendOK(t, store, upload(initial))
	for _, change := range []func(*Observation){func(r *Observation) { r.Data["status"] = json.RawMessage(`"failed"`) }, func(r *Observation) { r.RecordID = "new-record" }, func(r *Observation) { r.RecordSequence = 22; r.RecordID = "new-record" }} {
		raw, _ := json.Marshal(initial)
		var conflicting Observation
		_ = json.Unmarshal(raw, &conflicting)
		change(&conflicting)
		newRecord := observation(100, "conversation", "other", 0, Context{}, `{"status":"active"}`)
		if _, err := store.Append(ctx, upload(newRecord, conflicting)); !errors.Is(err, ErrConflict) {
			t.Fatalf("conflict = %v", err)
		}
		page, err := service.List(ctx, Query{})
		if err != nil || page.Total != 1 {
			t.Fatalf("transaction leaked: total=%d %v", page.Total, err)
		}
	}
	var wg sync.WaitGroup
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := store.Append(ctx, upload(initial)); err != nil {
				t.Errorf("retry = %v", err)
			}
		}()
	}
	wg.Wait()
	other := upload(initial)
	other.Device.InstallationID = "install-b"
	appendOK(t, store, other)
	page, err := service.List(ctx, Query{})
	if err != nil || page.Total != 2 {
		t.Fatalf("installation isolation: total=%d %v", page.Total, err)
	}
}
func TestLateParentsLatestRevisionCurrentMetricsAndTerminalProtection(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	appendOK(t, store, upload(batchObservation(1, "attempt", "native", 0, float64(0))))
	o, err := service.Overview(ctx, Query{})
	if err != nil || o.Coverage.UnresolvedRecordCount != 1 || o.Series[0].Mean == nil || *o.Series[0].Mean != 0 {
		t.Fatalf("unlinked zero: %+v %v", o.Coverage, err)
	}
	config := observation(2, "configuration", "config", 0, Context{}, `{"asr":{"id":"whisper","name":"Whisper tiny","sha256":"abc"},"reasoning":{"id":"qwen","name":"Qwen 0.5B","sha256":"def"},"settings":{},"runtime":{}}`)
	conversation := observation(3, "conversation", "chat", 0, Context{ConfigurationID: "config"}, `{"status":"active","source":"manual","started_at_ms":1000}`)
	turn := observation(4, "turn", "turn", 0, Context{ConversationID: "chat"}, `{"status":"running"}`)
	attempt := observation(5, "attempt", "attempt", 0, Context{TurnID: "turn"}, `{"status":"running","stage":"transcription"}`)
	appendOK(t, store, upload(config, conversation, turn, attempt))
	o, err = service.Overview(ctx, Query{ConversationID: "chat"})
	if err != nil || o.Counts.Conversations.Total != 1 || o.Counts.Attempts.Total != 1 || o.Coverage.UnresolvedRecordCount != 0 || o.CurrentConversation == nil || o.CurrentConversation.EventCount != 1 {
		t.Fatalf("late parents: %+v %v", o, err)
	}
	var model map[string]string
	_ = json.Unmarshal(o.CurrentConversation.Configuration["reasoning"], &model)
	if model["name"] != "Qwen 0.5B" || model["sha256"] != "def" {
		t.Fatalf("models lost: %+v", model)
	}
	appendOK(t, store, upload(observation(6, "attempt", "attempt", 2, Context{TurnID: "turn"}, `{"status":"succeeded","stage":"transcription"}`)))
	appendOK(t, store, upload(observation(7, "attempt", "attempt", 1, Context{TurnID: "turn"}, `{"status":"running","stage":"transcription"}`)))
	o, err = service.Overview(ctx, Query{})
	if err != nil || o.Counts.Attempts.Outcomes["succeeded"] != 1 || o.Counts.Attempts.Outcomes["running"] != 0 {
		t.Fatalf("old revision resurrected work: %+v %v", o.Counts, err)
	}
	reopened := observation(8, "attempt", "attempt", 3, Context{TurnID: "turn"}, `{"status":"running","stage":"transcription"}`)
	if _, err := store.Append(ctx, upload(reopened)); !errors.Is(err, ErrConflict) {
		t.Fatalf("terminal reopened: %v", err)
	}
}
func TestSnapshotPaginationFiltersMoreThanOneHundred(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	records := []Observation{}
	for i := range 137 {
		records = append(records, observation(i, "conversation", fmt.Sprintf("chat-%d", i), 0, Context{}, `{"status":"active","source":"manual"}`))
	}
	appendOK(t, store, upload(records...))
	first, err := service.List(ctx, Query{Limit: 100, Source: "manual"})
	if err != nil || first.Total != 137 || len(first.Records) != 100 || first.NextCursor == nil {
		t.Fatalf("first page: total=%d len=%d %v", first.Total, len(first.Records), err)
	}
	appendOK(t, store, upload(observation(200, "conversation", "new-chat", 0, Context{}, `{"status":"active","source":"manual"}`)))
	second, err := service.List(ctx, Query{Limit: 100, Source: "manual", Cursor: *first.NextCursor})
	if err != nil || second.Total != 137 || len(second.Records) != 37 || second.NextCursor != nil || second.AsOf != first.AsOf {
		t.Fatalf("snapshot drifted: total=%d %v", second.Total, err)
	}
	o, err := service.Overview(ctx, Query{AsOf: first.AsOf, Source: "manual"})
	if err != nil || o.Counts.Conversations.Total != 137 || len(o.RecentConversations) != 20 {
		t.Fatalf("overview truncated: %+v %v", o.Counts, err)
	}
	if _, err := service.List(ctx, Query{Cursor: *first.NextCursor, Source: "sweep"}); !errors.Is(err, ErrInvalidQuery) {
		t.Fatalf("changed filter accepted: %v", err)
	}
	fresh, _ := service.List(ctx, Query{})
	if _, err := service.List(ctx, Query{Cursor: *first.NextCursor, Source: "manual", AsOf: fresh.AsOf}); !errors.Is(err, ErrInvalidQuery) {
		t.Fatalf("changed watermark accepted: %v", err)
	}
	since, until := int64(1100), int64(1110)
	page, err := service.List(ctx, Query{SinceMS: &since, UntilMS: &until})
	if err != nil || page.Total != 11 {
		t.Fatalf("inclusive date filter: total=%d %v", page.Total, err)
	}
}
func TestLegacySnapshotFullTotalsExactDedupAndUnknowns(t *testing.T) {
	store, service := testService(t)
	legacy := phonemetrics.NewStore(store.db)
	ctx := context.Background()
	for i := range 125 {
		if _, err := legacy.Append(ctx, phonemetrics.Upload{SchemaVersion: 1, Device: device(), Batch: batch("legacy", int64(i), float64(i))}); err != nil {
			t.Fatal(err)
		}
	}
	o, err := service.Overview(ctx, Query{})
	if err != nil || o.Coverage.MetricBatchCount != 125 || o.Coverage.EventCount != 125 || o.Counts.Conversations.Total != 0 || o.Coverage.LegacyConversationCount != nil || o.Series[0].NumericCount != 125 || *o.Series[0].Mean != 62 {
		t.Fatalf("legacy summary: %+v %v", o.Coverage, err)
	}
	first, err := service.ListLegacy(ctx, Query{Limit: 100, AsOf: o.AsOf})
	if err != nil || first.Total != 125 || len(first.Batches) != 100 {
		t.Fatalf("legacy page: total=%d %v", first.Total, err)
	}
	if _, err := legacy.Append(ctx, phonemetrics.Upload{SchemaVersion: 1, Device: device(), Batch: batch("legacy", 125, float64(125))}); err != nil {
		t.Fatal(err)
	}
	appendOK(t, store, upload(batchObservation(1000, "attempt", "legacy", 0, float64(0))))
	second, err := service.ListLegacy(ctx, Query{Limit: 100, Cursor: *first.NextCursor})
	if err != nil || second.Total != 125 || len(second.Batches) != 25 {
		t.Fatalf("legacy watermark: total=%d %v", second.Total, err)
	}
	fresh, err := service.Overview(ctx, Query{})
	if err != nil || fresh.Coverage.MetricBatchCount != 126 || fresh.Coverage.LegacyBatchCount != 125 || fresh.Coverage.EventCount != 126 {
		t.Fatalf("dual transport doubled metrics: %+v %v", fresh.Coverage, err)
	}
	filtered, err := service.Overview(ctx, Query{InstallationID: "other"})
	if err != nil || filtered.Coverage.EventCount != 0 {
		t.Fatalf("installation filter: %+v %v", filtered.Coverage, err)
	}
	oldAgain, err := service.Overview(ctx, Query{AsOf: o.AsOf})
	if err != nil || oldAgain.Coverage.MetricBatchCount != 125 {
		t.Fatalf("snapshot drifted: %+v %v", oldAgain.Coverage, err)
	}
}
func TestGroupingKeepsStatusUnavailableAndInstallationsSeparate(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	for i := range 2 {
		u := upload(batchObservation(i, "attempt", "native", 0, float64(0)))
		u.Device.InstallationID = fmt.Sprintf("install-%d", i)
		appendOK(t, store, u)
	}
	b := batch("status", 0, "nominal")
	b.Events[0].Name = "thermal"
	b.Events[0].Unit = "status"
	b.Events = append(b.Events, metrics.Event{SchemaVersion: 1, RunID: "status", Sequence: 1, TimestampMS: 1000, Name: "power", Value: nil, Unit: "watts", Scope: "device", Source: "fixture", UnavailableReason: "no meter"})
	raw, _ := json.Marshal(map[string]any{"batch": b})
	appendOK(t, store, upload(observation(10, "metric_batch", "status", 0, Context{AttemptID: "missing"}, string(raw))))
	o, err := service.Overview(ctx, Query{})
	if err != nil || len(o.Series) != 4 {
		t.Fatalf("series merged: %+v %v", o.Series, err)
	}
	for _, s := range o.Series {
		switch s.Name {
		case "duration_ms":
			if s.NumericCount != 1 || s.Mean == nil || *s.Mean != 0 {
				t.Fatal(s)
			}
		case "thermal":
			if s.NumericCount != 0 || s.StatusCounts["nominal"] != 1 {
				t.Fatal(s)
			}
		case "power":
			if s.UnavailableCount != 1 || s.Mean != nil {
				t.Fatal(s)
			}
		}
	}
}

func TestFinalizationRequiresManifestReceiptAndCurrentTargetRevision(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	attempt := observation(1, "attempt", "attempt", 1, Context{}, `{"status":"succeeded","stage":"transcription"}`)
	metric := batchObservation(2, "attempt", "native", 0, float64(5))
	final := observation(3, "finalization", "attempt-final", 0, Context{AttemptID: "attempt"}, `{"status":"complete","target_revision":1,"required_record_ids":["record-1","record-2"],"record_count":2,"event_count":1,"dropped_events":0}`)
	appendOK(t, store, upload(attempt, final))
	o, err := service.Overview(ctx, Query{})
	if err != nil || o.Coverage.Finalization.Complete != 0 || o.RecentRuns[0].Completeness != "partial" {
		t.Fatalf("early finalization falsely complete: %+v %v", o.Coverage, err)
	}
	before := o.AsOf
	appendOK(t, store, upload(metric))
	o, err = service.Overview(ctx, Query{})
	if err != nil || o.Coverage.Finalization.Complete != 1 || o.RecentRuns[0].Completeness != "complete" {
		t.Fatalf("manifest arrived: %+v %v", o.Coverage, err)
	}
	old, err := service.Overview(ctx, Query{AsOf: before})
	if err != nil || old.Coverage.Finalization.Complete != 0 {
		t.Fatalf("snapshot complete changed retroactively: %+v %v", old.Coverage, err)
	}
	revised := observation(4, "attempt", "attempt", 2, Context{}, `{"status":"succeeded","stage":"transcription","duration_ms":5}`)
	appendOK(t, store, upload(revised))
	o, err = service.Overview(ctx, Query{})
	if err != nil || o.Coverage.Finalization.Complete != 0 || o.RecentRuns[0].Completeness != "partial" {
		t.Fatalf("old revision finalization certified new target: %+v %v", o.Coverage, err)
	}
}

func TestOverviewDoesNotRepeatHierarchicalDroppedEvents(t *testing.T) {
	store, service := testService(t)
	appendOK(t, store, upload(
		observation(1, "sweep", "s", 1, Context{}, `{"status":"succeeded"}`),
		observation(2, "conversation", "c", 1, Context{SweepID: "s"}, `{"status":"succeeded"}`),
		observation(3, "attempt", "a", 1, Context{ConversationID: "c"}, `{"status":"succeeded"}`),
		observation(4, "finalization", "fa", 1, Context{AttemptID: "a"}, `{"status":"partial","event_count":0,"dropped_events":3}`),
		observation(5, "finalization", "fc", 1, Context{ConversationID: "c"}, `{"status":"partial","event_count":0,"dropped_events":3}`),
		observation(6, "finalization", "fs", 1, Context{SweepID: "s"}, `{"status":"partial","event_count":0,"dropped_events":3}`),
	))
	o, err := service.Overview(context.Background(), Query{})
	if err != nil || o.Coverage.Finalization.DroppedEvents != 3 {
		t.Fatalf("nested drops = %d, %v", o.Coverage.Finalization.DroppedEvents, err)
	}
	appendOK(t, store, upload(
		observation(7, "attempt", "b", 1, Context{ConversationID: "c"}, `{"status":"failed"}`),
		observation(8, "finalization", "fb", 1, Context{AttemptID: "b"}, `{"status":"partial","event_count":0,"dropped_events":2}`),
	))
	o, err = service.Overview(context.Background(), Query{})
	if err != nil || o.Coverage.Finalization.DroppedEvents != 5 {
		t.Fatalf("disjoint child drops = %d, %v", o.Coverage.Finalization.DroppedEvents, err)
	}
}

func TestUnavailableInterruptedDurationRemainsNull(t *testing.T) {
	store, service := testService(t)
	appendOK(t, store, upload(observation(1, "conversation", "interrupted", 1, Context{}, `{"status":"interrupted","duration_ms":null}`)))
	page, err := service.List(context.Background(), Query{})
	if err != nil || len(page.Records) != 1 || string(page.Records[0].Record.Data["duration_ms"]) != "null" {
		t.Fatalf("unavailable duration changed: %+v, %v", page, err)
	}
}
func TestBatchConflictsAndDistinctAttemptsKeepNativeCountersSeparate(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	appendOK(t, store, upload(batchObservation(1, "attempt-a", "native-1", 0, float64(5))))
	conflict := batchObservation(2, "attempt-a", "native-1", 0, float64(8))
	if _, err := store.Append(ctx, upload(conflict)); !errors.Is(err, ErrConflict) {
		t.Fatalf("batch identity conflict: %v", err)
	}
	appendOK(t, store, upload(batchObservation(3, "attempt-b", "native-1", 0, float64(5))))
	o, err := service.Overview(ctx, Query{})
	if err != nil || o.Coverage.EventCount != 2 {
		t.Fatalf("native restart samples collapsed: %+v %v", o.Coverage, err)
	}
}
func TestFilteredReadIdentifiesSupportAncestors(t *testing.T) {
	store, service := testService(t)
	ctx := context.Background()
	appendOK(t, store, upload(observation(1, "conversation", "chat", 0, Context{}, `{"status":"active","source":"manual"}`), observation(2, "attempt", "attempt", 0, Context{ConversationID: "chat"}, `{"status":"succeeded","stage":"reasoning"}`), batchObservation(3, "attempt", "native", 0, float64(0))))
	p, err := service.List(ctx, Query{Stage: "reasoning"})
	if err != nil || len(p.Records) != 3 || !p.Filtered || len(p.SelectedRecordKeys) != 2 {
		t.Fatalf("selected vs support: %+v %v", p, err)
	}
	for _, k := range p.SelectedRecordKeys {
		if k.RecordID == "record-1" {
			t.Fatal("support conversation marked selected")
		}
	}
}
