package metrics

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"reflect"
	"strings"
)

// CurrentSchemaVersion is the version of the JSON metrics contract understood
// by this package.
const CurrentSchemaVersion int64 = 1

// SchemaVersion is kept as a concise name for callers constructing batches.
const SchemaVersion = CurrentSchemaVersion

// MetricSchemaVersion mirrors the name used by the Rust metrics crate.
const MetricSchemaVersion = CurrentSchemaVersion

// Value is a JSON scalar supported by a metric event. Numbers are decoded by
// encoding/json as float64; strings, booleans, and nil are also supported.
type Value = any

// MetricValue is an alternate name for Value used by callers that share the
// Rust terminology.
type MetricValue = Value

// Event is one versioned metric measurement produced during a run.
type Event struct {
	SchemaVersion     int64  `json:"schema_version"`
	ExperimentID      string `json:"experiment_id,omitempty"`
	RunID             string `json:"run_id"`
	Sequence          int64  `json:"sequence"`
	TimestampMS       int64  `json:"timestamp_ms"`
	Name              string `json:"name"`
	Value             Value  `json:"value"`
	Unit              string `json:"unit"`
	Scope             string `json:"scope"`
	Source            string `json:"source"`
	UnavailableReason string `json:"unavailable_reason,omitempty"`

	decoded                  bool
	schemaVersionPresent     bool
	runIDPresent             bool
	sequencePresent          bool
	timestampPresent         bool
	namePresent              bool
	valuePresent             bool
	unitPresent              bool
	scopePresent             bool
	sourcePresent            bool
	unavailableReasonPresent bool
}

// Batch is the versioned envelope stored by the metrics package. Its storage
// is intentionally independent from benchmark.Experiment.Result.
type Batch struct {
	SchemaVersion  int64   `json:"schema_version"`
	ExperimentID   string  `json:"experiment_id,omitempty"`
	RunID          string  `json:"run_id"`
	SequenceNumber int64   `json:"sequence_number"`
	TimestampMS    int64   `json:"timestamp_ms"`
	Events         []Event `json:"events"`

	decoded              bool
	schemaVersionPresent bool
	runIDPresent         bool
	sequencePresent      bool
	timestampPresent     bool
	eventsPresent        bool
}

// MetricEvent and MetricBatch are compatibility names for code using the Rust
// metric terminology.
type MetricEvent = Event
type MetricBatch = Batch

var (
	ErrInvalidBatch = errors.New("invalid metrics batch")
	ErrInvalidEvent = errors.New("invalid metric event")

	ErrInvalidSchemaVersion      = errors.New("invalid metrics schema version")
	ErrUnsupportedSchemaVersion  = ErrInvalidSchemaVersion
	ErrRunIDRequired             = errors.New("run_id is required")
	ErrEventsRequired            = errors.New("events is required")
	ErrEventRunIDRequired        = errors.New("event run_id is required")
	ErrEventSequenceRequired     = errors.New("event sequence is required")
	ErrEventTimestampRequired    = errors.New("event timestamp_ms is required")
	ErrExperimentIDRequired      = errors.New("experiment_id is required")
	ErrExperimentIDMismatch      = errors.New("experiment_id does not match")
	ErrDuplicateEventSequence    = errors.New("duplicate event sequence")
	ErrInvalidMetricValue        = errors.New("metric value must be a number, string, boolean, or null")
	ErrUnavailableReason         = errors.New("unavailable_reason is required for an unavailable value")
	ErrMetricsRepositoryFailure  = errors.New("metrics repository unavailable")
	ErrMetricsBatchNotFound      = errors.New("metrics batch not found")
	ErrRepositoryReadUnsupported = errors.New("metrics repository does not support retrieval")
)

// UnmarshalJSON records field presence so required zero-valued numeric fields
// can be distinguished from omitted fields, and explicit null metric values
// remain valid values.
func (e *Event) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	var wire struct {
		SchemaVersion     *int64  `json:"schema_version"`
		ExperimentID      *string `json:"experiment_id"`
		RunID             *string `json:"run_id"`
		Sequence          *int64  `json:"sequence"`
		TimestampMS       *int64  `json:"timestamp_ms"`
		Name              *string `json:"name"`
		Unit              *string `json:"unit"`
		Scope             *string `json:"scope"`
		Source            *string `json:"source"`
		UnavailableReason *string `json:"unavailable_reason"`
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}

	decoded := Event{decoded: true}
	if wire.SchemaVersion != nil {
		decoded.SchemaVersion = *wire.SchemaVersion
		decoded.schemaVersionPresent = true
	}
	if wire.ExperimentID != nil {
		decoded.ExperimentID = *wire.ExperimentID
	}
	if wire.RunID != nil {
		decoded.RunID = *wire.RunID
		decoded.runIDPresent = true
	}
	if wire.Sequence != nil {
		decoded.Sequence = *wire.Sequence
		decoded.sequencePresent = true
	}
	if wire.TimestampMS != nil {
		decoded.TimestampMS = *wire.TimestampMS
		decoded.timestampPresent = true
	}
	if wire.Name != nil {
		decoded.Name = *wire.Name
		decoded.namePresent = true
	}
	if wire.Unit != nil {
		decoded.Unit = *wire.Unit
		decoded.unitPresent = true
	}
	if wire.Scope != nil {
		decoded.Scope = *wire.Scope
		decoded.scopePresent = true
	}
	if wire.Source != nil {
		decoded.Source = *wire.Source
		decoded.sourcePresent = true
	}
	if wire.UnavailableReason != nil {
		decoded.UnavailableReason = *wire.UnavailableReason
		decoded.unavailableReasonPresent = true
	}

	if value, ok := raw["value"]; ok {
		decoded.valuePresent = true
		if isJSONNull(value) {
			decoded.Value = nil
		} else if err := json.Unmarshal(value, &decoded.Value); err != nil {
			return err
		}
	}

	*e = decoded
	return nil
}

// UnmarshalJSON records required batch-field presence. In addition to the
// canonical names, sequence and timestamp aliases are accepted when reading a
// producer that uses the shorter Rust-style names.
func (b *Batch) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	var wire struct {
		SchemaVersion  *int64  `json:"schema_version"`
		ExperimentID   *string `json:"experiment_id"`
		RunID          *string `json:"run_id"`
		SequenceNumber *int64  `json:"sequence_number"`
		Sequence       *int64  `json:"sequence"`
		BatchSequence  *int64  `json:"batch_sequence"`
		TimestampMS    *int64  `json:"timestamp_ms"`
		Timestamp      *int64  `json:"timestamp"`
		Events         []Event `json:"events"`
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}

	sequence, sequencePresent, sequenceConflict := selectInt64(
		wire.SequenceNumber,
		wire.Sequence,
		wire.BatchSequence,
	)
	if sequenceConflict {
		return errors.New("conflicting batch sequence fields")
	}
	timestamp, timestampPresent, timestampConflict := selectInt64(
		wire.TimestampMS,
		wire.Timestamp,
	)
	if timestampConflict {
		return errors.New("conflicting batch timestamp fields")
	}

	decoded := Batch{decoded: true}
	if wire.SchemaVersion != nil {
		decoded.SchemaVersion = *wire.SchemaVersion
		decoded.schemaVersionPresent = true
	}
	if wire.ExperimentID != nil {
		decoded.ExperimentID = *wire.ExperimentID
	}
	if wire.RunID != nil {
		decoded.RunID = *wire.RunID
		decoded.runIDPresent = true
	}
	if sequencePresent {
		decoded.SequenceNumber = sequence
		decoded.sequencePresent = true
	}
	if timestampPresent {
		decoded.TimestampMS = timestamp
		decoded.timestampPresent = true
	}
	if _, ok := raw["events"]; ok {
		decoded.eventsPresent = true
		if !isJSONNull(raw["events"]) {
			decoded.Events = wire.Events
		}
	}

	*b = decoded
	return nil
}

// Validate checks the batch envelope, event identity, experiment consistency,
// and scalar-value invariants.
func (b Batch) Validate() error {
	if b.decoded && !b.schemaVersionPresent {
		return invalidBatch(ErrInvalidSchemaVersion)
	}
	if b.SchemaVersion != CurrentSchemaVersion {
		return fmt.Errorf("%w: %w: got %d, want %d", ErrInvalidBatch, ErrInvalidSchemaVersion, b.SchemaVersion, CurrentSchemaVersion)
	}
	if (b.decoded && !b.runIDPresent) || isBlank(b.RunID) {
		return invalidBatch(ErrRunIDRequired)
	}
	if b.decoded && !b.sequencePresent {
		return invalidBatch(errors.New("sequence_number is required"))
	}
	if b.SequenceNumber < 0 {
		return invalidBatch(errors.New("sequence_number must be non-negative"))
	}
	if b.decoded && !b.timestampPresent {
		return invalidBatch(errors.New("timestamp_ms is required"))
	}
	if b.TimestampMS < 0 {
		return invalidBatch(errors.New("timestamp_ms must be non-negative"))
	}
	if b.decoded && !b.eventsPresent {
		return invalidBatch(ErrEventsRequired)
	}
	if b.Events == nil {
		return invalidBatch(ErrEventsRequired)
	}
	if b.ExperimentID != "" && isBlank(b.ExperimentID) {
		return invalidBatch(ErrExperimentIDRequired)
	}

	seenSequences := make(map[int64]struct{}, len(b.Events))
	experimentID := b.ExperimentID
	for index, event := range b.Events {
		if _, exists := seenSequences[event.Sequence]; exists {
			return fmt.Errorf("%w: events[%d]: %w", ErrInvalidBatch, index, ErrDuplicateEventSequence)
		}
		seenSequences[event.Sequence] = struct{}{}

		if err := event.validateForBatch(b.RunID); err != nil {
			return fmt.Errorf("%w: events[%d]: %w", ErrInvalidBatch, index, err)
		}
		if event.ExperimentID == "" {
			continue
		}
		if isBlank(event.ExperimentID) {
			return fmt.Errorf("%w: events[%d]: %w", ErrInvalidBatch, index, ErrExperimentIDRequired)
		}
		if experimentID == "" {
			experimentID = event.ExperimentID
		} else if event.ExperimentID != experimentID {
			return fmt.Errorf("%w: events[%d]: %w", ErrInvalidBatch, index, ErrExperimentIDMismatch)
		}
	}

	return nil
}

// ValidateForExperiment validates a batch against an externally supplied
// experiment identifier. The batch identifier may be omitted, but a supplied
// identifier must agree with the external value.
func (b Batch) ValidateForExperiment(experimentID string) error {
	if isBlank(experimentID) {
		return fmt.Errorf("%w: %w", ErrInvalidBatch, ErrExperimentIDRequired)
	}
	if b.ExperimentID != "" && b.ExperimentID != experimentID {
		return fmt.Errorf("%w: %w", ErrInvalidBatch, ErrExperimentIDMismatch)
	}
	return b.Validate()
}

// Validate checks an event without a containing batch.
func (e Event) Validate() error {
	return e.validateForBatch("")
}

func (e Event) validateForBatch(expectedRunID string) error {
	if e.decoded && !e.schemaVersionPresent {
		return invalidEvent(ErrInvalidSchemaVersion)
	}
	if e.SchemaVersion != CurrentSchemaVersion {
		return fmt.Errorf("%w: %w: got %d, want %d", ErrInvalidEvent, ErrInvalidSchemaVersion, e.SchemaVersion, CurrentSchemaVersion)
	}
	if (e.decoded && !e.runIDPresent) || isBlank(e.RunID) {
		return invalidEvent(ErrEventRunIDRequired)
	}
	if expectedRunID != "" && e.RunID != expectedRunID {
		return invalidEvent(errors.New("event run_id does not match batch run_id"))
	}
	if e.decoded && !e.sequencePresent {
		return invalidEvent(ErrEventSequenceRequired)
	}
	if e.Sequence < 0 {
		return invalidEvent(errors.New("event sequence must be non-negative"))
	}
	if e.decoded && !e.timestampPresent {
		return invalidEvent(ErrEventTimestampRequired)
	}
	if e.TimestampMS < 0 {
		return invalidEvent(errors.New("event timestamp_ms must be non-negative"))
	}
	if (e.decoded && !e.namePresent) || isBlank(e.Name) {
		return invalidEvent(errors.New("event name is required"))
	}
	if (e.decoded && !e.valuePresent) || !isSupportedMetricValue(e.Value) {
		if e.decoded && !e.valuePresent {
			return invalidEvent(errors.New("event value is required"))
		}
		return invalidEvent(ErrInvalidMetricValue)
	}
	if (e.decoded && !e.unitPresent) || isBlank(e.Unit) {
		return invalidEvent(errors.New("event unit is required"))
	}
	if (e.decoded && !e.scopePresent) || isBlank(e.Scope) {
		return invalidEvent(errors.New("event scope is required"))
	}
	if (e.decoded && !e.sourcePresent) || isBlank(e.Source) {
		return invalidEvent(errors.New("event source is required"))
	}
	if e.ExperimentID != "" && isBlank(e.ExperimentID) {
		return invalidEvent(ErrExperimentIDRequired)
	}
	if e.Value == nil {
		if isBlank(e.UnavailableReason) {
			return invalidEvent(ErrUnavailableReason)
		}
	} else if e.UnavailableReason != "" || e.unavailableReasonPresent {
		return invalidEvent(errors.New("unavailable_reason requires a null value"))
	}

	return nil
}

func invalidBatch(reason error) error {
	return fmt.Errorf("%w: %w", ErrInvalidBatch, reason)
}

func invalidEvent(reason error) error {
	return fmt.Errorf("%w: %w", ErrInvalidEvent, reason)
}

func isBlank(value string) bool {
	return strings.TrimSpace(value) == ""
}

func isJSONNull(value []byte) bool {
	return bytes.Equal(bytes.TrimSpace(value), []byte("null"))
}

func selectInt64(values ...*int64) (int64, bool, bool) {
	var selected int64
	present := false
	for _, value := range values {
		if value == nil {
			continue
		}
		if present && selected != *value {
			return 0, true, true
		}
		selected = *value
		present = true
	}
	return selected, present, false
}

func isSupportedMetricValue(value Value) bool {
	if value == nil {
		return true
	}

	if number, ok := value.(json.Number); ok {
		if number == "" || !json.Valid([]byte(number)) {
			return false
		}
		_, err := number.Float64()
		return err == nil
	}

	reflected := reflect.ValueOf(value)
	switch reflected.Kind() {
	case reflect.String, reflect.Bool:
		return true
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return true
	case reflect.Float32, reflect.Float64:
		floatValue := reflected.Float()
		return !math.IsNaN(floatValue) && !math.IsInf(floatValue, 0)
	default:
		return false
	}
}
