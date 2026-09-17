package metrics

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestBatchJSONSupportsRustMetricEventScalars(t *testing.T) {
	payload := `{
		"schema_version":1,
		"experiment_id":"exp-1",
		"run_id":"run-1",
		"sequence_number":4,
		"timestamp_ms":1000,
		"events":[
			{"schema_version":1,"experiment_id":"exp-1","run_id":"run-1","sequence":0,"timestamp_ms":1000,"name":"temperature","value":21.5,"unit":"celsius","scope":"resource","source":"sensor"},
			{"schema_version":1,"experiment_id":"exp-1","run_id":"run-1","sequence":1,"timestamp_ms":1001,"name":"phase","value":"warmup","unit":"status","scope":"run","source":"workflow"},
			{"schema_version":1,"experiment_id":"exp-1","run_id":"run-1","sequence":2,"timestamp_ms":1002,"name":"available","value":true,"unit":"boolean","scope":"run","source":"workflow"},
			{"schema_version":1,"experiment_id":"exp-1","run_id":"run-1","sequence":3,"timestamp_ms":1003,"name":"power","value":null,"unit":"watts","scope":"resource","source":"sensor","unavailable_reason":"not measured"}
		]
	}`

	var batch Batch
	if err := json.Unmarshal([]byte(payload), &batch); err != nil {
		t.Fatalf("json.Unmarshal returned error: %v", err)
	}
	if err := batch.Validate(); err != nil {
		t.Fatalf("Validate returned error: %v", err)
	}
	if batch.ExperimentID != "exp-1" || batch.RunID != "run-1" {
		t.Fatalf("batch identity = (%q, %q), want (exp-1, run-1)", batch.ExperimentID, batch.RunID)
	}
	if batch.SequenceNumber != 4 || batch.TimestampMS != 1000 {
		t.Fatalf("batch metadata = (%d, %d), want (4, 1000)", batch.SequenceNumber, batch.TimestampMS)
	}
	if len(batch.Events) != 4 {
		t.Fatalf("event count = %d, want 4", len(batch.Events))
	}
	if _, ok := batch.Events[0].Value.(float64); !ok {
		t.Fatalf("number value has type %T, want float64", batch.Events[0].Value)
	}
	if batch.Events[1].Value != "warmup" {
		t.Fatalf("string value = %#v, want warmup", batch.Events[1].Value)
	}
	if batch.Events[2].Value != true {
		t.Fatalf("boolean value = %#v, want true", batch.Events[2].Value)
	}
	if batch.Events[3].Value != nil {
		t.Fatalf("null value = %#v, want nil", batch.Events[3].Value)
	}

	encoded, err := json.Marshal(batch)
	if err != nil {
		t.Fatalf("json.Marshal returned error: %v", err)
	}
	var roundTrip Batch
	if err := json.Unmarshal(encoded, &roundTrip); err != nil {
		t.Fatalf("round-trip unmarshal returned error: %v", err)
	}
	if err := roundTrip.Validate(); err != nil {
		t.Fatalf("round-trip Validate returned error: %v", err)
	}
}

func TestBatchJSONAcceptsShortRustStyleBatchMetadata(t *testing.T) {
	payload := `{"schema_version":1,"run_id":"run-1","sequence":0,"timestamp":0,"events":[]}`

	var batch Batch
	if err := json.Unmarshal([]byte(payload), &batch); err != nil {
		t.Fatalf("json.Unmarshal returned error: %v", err)
	}
	if err := batch.Validate(); err != nil {
		t.Fatalf("Validate returned error: %v", err)
	}
	if batch.SequenceNumber != 0 || batch.TimestampMS != 0 {
		t.Fatalf("metadata = (%d, %d), want (0, 0)", batch.SequenceNumber, batch.TimestampMS)
	}
}

func TestBatchValidationRejectsMissingIdentityAndInconsistentEvents(t *testing.T) {
	tests := []struct {
		name    string
		batch   Batch
		wantErr error
	}{
		{
			name:    "missing run id",
			batch:   Batch{SchemaVersion: CurrentSchemaVersion, SequenceNumber: 0, TimestampMS: 1, Events: []Event{}},
			wantErr: ErrRunIDRequired,
		},
		{
			name:    "missing event list",
			batch:   Batch{SchemaVersion: CurrentSchemaVersion, RunID: "run-1", SequenceNumber: 0, TimestampMS: 1},
			wantErr: ErrEventsRequired,
		},
		{
			name: "missing event run id",
			batch: Batch{
				SchemaVersion:  CurrentSchemaVersion,
				RunID:          "run-1",
				SequenceNumber: 0,
				TimestampMS:    1,
				Events:         []Event{{SchemaVersion: CurrentSchemaVersion, Sequence: 0, TimestampMS: 1, Name: "latency", Value: 1, Unit: "ms", Scope: "run", Source: "test"}},
			},
			wantErr: ErrEventRunIDRequired,
		},
		{
			name: "experiment mismatch",
			batch: Batch{
				SchemaVersion:  CurrentSchemaVersion,
				ExperimentID:   "exp-1",
				RunID:          "run-1",
				SequenceNumber: 0,
				TimestampMS:    1,
				Events:         []Event{{SchemaVersion: CurrentSchemaVersion, ExperimentID: "exp-2", RunID: "run-1", Sequence: 0, TimestampMS: 1, Name: "latency", Value: 1, Unit: "ms", Scope: "run", Source: "test"}},
			},
			wantErr: ErrExperimentIDMismatch,
		},
		{
			name: "duplicate event sequence",
			batch: Batch{
				SchemaVersion:  CurrentSchemaVersion,
				RunID:          "run-1",
				SequenceNumber: 0,
				TimestampMS:    1,
				Events: []Event{
					{SchemaVersion: CurrentSchemaVersion, RunID: "run-1", Sequence: 0, TimestampMS: 1, Name: "one", Value: 1, Unit: "count", Scope: "run", Source: "test"},
					{SchemaVersion: CurrentSchemaVersion, RunID: "run-1", Sequence: 0, TimestampMS: 2, Name: "two", Value: 2, Unit: "count", Scope: "run", Source: "test"},
				},
			},
			wantErr: ErrDuplicateEventSequence,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := test.batch.Validate(); !errors.Is(err, test.wantErr) {
				t.Fatalf("Validate error = %v, want errors.Is(..., %v)", err, test.wantErr)
			}
		})
	}
}

func TestEventValidationRequiresUnavailableReasonForNull(t *testing.T) {
	event := validEvent()
	event.Value = nil
	if err := event.Validate(); !errors.Is(err, ErrUnavailableReason) {
		t.Fatalf("Validate error = %v, want ErrUnavailableReason", err)
	}

	event.UnavailableReason = "sensor unavailable"
	if err := event.Validate(); err != nil {
		t.Fatalf("Validate returned error for unavailable event: %v", err)
	}
}

func TestEventValidationRejectsObjectsAsValues(t *testing.T) {
	event := validEvent()
	event.Value = map[string]any{"kind": "unsupported"}
	if err := event.Validate(); !errors.Is(err, ErrInvalidMetricValue) {
		t.Fatalf("Validate error = %v, want ErrInvalidMetricValue", err)
	}
}

func validBatch() Batch {
	return Batch{
		SchemaVersion:  CurrentSchemaVersion,
		ExperimentID:   "exp-1",
		RunID:          "run-1",
		SequenceNumber: 0,
		TimestampMS:    1000,
		Events:         []Event{validEvent()},
	}
}

func validEvent() Event {
	return Event{
		SchemaVersion: CurrentSchemaVersion,
		ExperimentID:  "exp-1",
		RunID:         "run-1",
		Sequence:      0,
		TimestampMS:   1000,
		Name:          "latency",
		Value:         12.5,
		Unit:          "ms",
		Scope:         "run",
		Source:        "test",
	}
}
