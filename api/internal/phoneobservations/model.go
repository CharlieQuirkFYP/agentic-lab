// Package phoneobservations stores immutable phone telemetry; it never executes inference.
package phoneobservations

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"strings"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
)

var (
	ErrInvalidUpload = errors.New("invalid phone observation upload")
	ErrConflict      = errors.New("phone observation identity conflict")
	ErrInvalidQuery  = errors.New("invalid phone observation query")
)

const MaxRecords = 500

type Context struct {
	ConfigurationID  string `json:"configuration_id,omitempty"`
	ConversationID   string `json:"conversation_id,omitempty"`
	TurnID           string `json:"turn_id,omitempty"`
	AttemptID        string `json:"attempt_id,omitempty"`
	SweepID          string `json:"sweep_id,omitempty"`
	JobID            string `json:"job_id,omitempty"`
	RuntimeSessionID string `json:"runtime_session_id,omitempty"`
}
type Observation struct {
	SchemaVersion    int                        `json:"schema_version"`
	RecordID         string                     `json:"record_id"`
	ProducerStreamID string                     `json:"producer_stream_id"`
	RecordSequence   int64                      `json:"record_sequence"`
	TimestampMS      int64                      `json:"timestamp_ms"`
	Kind             string                     `json:"kind"`
	EntityID         string                     `json:"entity_id"`
	Revision         int64                      `json:"revision"`
	Context          Context                    `json:"context"`
	Data             map[string]json.RawMessage `json:"data"`
}

func (r *Observation) UnmarshalJSON(data []byte) error {
	type wire Observation
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	for _, k := range []string{"schema_version", "record_id", "producer_stream_id", "record_sequence", "timestamp_ms", "kind", "entity_id", "revision", "context", "data"} {
		if v, ok := fields[k]; !ok || bytes.Equal(bytes.TrimSpace(v), []byte("null")) {
			return ErrInvalidUpload
		}
	}
	var v wire
	d := json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	if err := d.Decode(&v); err != nil {
		return err
	}
	*r = Observation(v)
	return nil
}

type Upload struct {
	SchemaVersion int                 `json:"schema_version"`
	Device        phonemetrics.Device `json:"device"`
	Records       []Observation       `json:"records"`
}
type Accepted struct {
	Status            string   `json:"status"`
	RecordCount       int      `json:"record_count"`
	AcceptedRecordIDs []string `json:"accepted_record_ids"`
}
type StoredRecord struct {
	Device       phonemetrics.Device `json:"device"`
	Record       Observation         `json:"record"`
	ReceivedAtMS int64               `json:"received_at_ms"`
	id           int64
}
type LegacyRecord struct {
	Device       *phonemetrics.Device `json:"device"`
	Batch        metrics.Batch        `json:"batch"`
	ReceivedAtMS int64                `json:"received_at_ms"`
	id           int64
}
type Page struct {
	SchemaVersion      int            `json:"schema_version"`
	AsOf               string         `json:"as_of"`
	Records            []StoredRecord `json:"records"`
	Total              int            `json:"total"`
	NextCursor         *string        `json:"next_cursor"`
	SelectedRecordKeys []RecordKey    `json:"selected_record_keys"`
	Filtered           bool           `json:"filtered"`
}

type RecordKey struct {
	InstallationID string `json:"installation_id"`
	RecordID       string `json:"record_id"`
}
type LegacyPage struct {
	SchemaVersion int            `json:"schema_version"`
	AsOf          string         `json:"as_of"`
	Batches       []LegacyRecord `json:"batches"`
	Total         int            `json:"total"`
	NextCursor    *string        `json:"next_cursor"`
}

func DecodeUpload(r io.Reader) (Upload, error) {
	var u Upload
	d := json.NewDecoder(r)
	d.DisallowUnknownFields()
	if err := d.Decode(&u); err != nil {
		return u, err
	}
	var extra any
	if err := d.Decode(&extra); err != io.EOF {
		if err != nil {
			return u, err
		}
		return u, ErrInvalidUpload
	}
	return u, u.Validate()
}
func validID(s string) bool {
	return strings.TrimSpace(s) != "" && len(s) <= 256 && !strings.ContainsRune(s, '\x00')
}
func (u Upload) Validate() error {
	if u.SchemaVersion != 1 || len(u.Records) == 0 || len(u.Records) > MaxRecords {
		return ErrInvalidUpload
	}
	for _, s := range []string{u.Device.InstallationID, u.Device.Hardware, u.Device.OSVersion, u.Device.AppVersion, u.Device.Build} {
		if !validID(s) {
			return ErrInvalidUpload
		}
	}
	for _, r := range u.Records {
		if err := r.Validate(); err != nil {
			return err
		}
	}
	return nil
}
func (r Observation) Validate() error {
	if r.SchemaVersion != 1 || !validID(r.RecordID) || !validID(r.ProducerStreamID) || !validID(r.EntityID) || r.RecordSequence < 0 || r.Revision < 0 || r.TimestampMS < 0 || r.Data == nil {
		return ErrInvalidUpload
	}
	for _, id := range []string{r.Context.ConfigurationID, r.Context.ConversationID, r.Context.TurnID, r.Context.AttemptID, r.Context.SweepID, r.Context.JobID, r.Context.RuntimeSessionID} {
		if id != "" && !validID(id) {
			return ErrInvalidUpload
		}
	}
	if containsContent(r.Data) {
		return ErrInvalidUpload
	}
	switch r.Kind {
	case "configuration":
		for _, k := range []string{"asr", "reasoning"} {
			v, ok := r.Data[k]
			if !ok {
				return ErrInvalidUpload
			}
			if string(v) != "null" {
				var model map[string]json.RawMessage
				if json.Unmarshal(v, &model) != nil || !validID(str(model, "id")) || !validID(str(model, "name")) || !validID(str(model, "sha256")) {
					return ErrInvalidUpload
				}
			}
		}
		for _, k := range []string{"settings", "runtime"} {
			var m map[string]json.RawMessage
			if json.Unmarshal(r.Data[k], &m) != nil || m == nil {
				return ErrInvalidUpload
			}
		}
	case "conversation", "turn", "attempt", "sweep", "job":
		if !validStatus(str(r.Data, "status"), r.Kind) {
			return ErrInvalidUpload
		}
	case "metric_batch":
		var b metrics.Batch
		if json.Unmarshal(r.Data["batch"], &b) != nil || b.Validate() != nil {
			return ErrInvalidUpload
		}
	case "finalization":
		if status := str(r.Data, "status"); status != "complete" && status != "partial" {
			return ErrInvalidUpload
		}
		if r.Context.ConversationID == "" && r.Context.AttemptID == "" && r.Context.SweepID == "" {
			return ErrInvalidUpload
		}
		for _, k := range []string{"event_count", "dropped_events"} {
			var n int64
			if json.Unmarshal(r.Data[k], &n) != nil || n < 0 {
				return ErrInvalidUpload
			}
		}
	default:
		return ErrInvalidUpload
	}
	for _, k := range []string{"started_at_ms", "finished_at_ms", "duration_ms", "turn_index", "repetition", "record_count", "event_count", "dropped_events"} {
		if raw, ok := r.Data[k]; ok {
			if k == "duration_ms" && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
				continue // Interrupted work may have no settled native duration.
			}
			var n float64
			if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) || json.Unmarshal(raw, &n) != nil || n < 0 {
				return ErrInvalidUpload
			}
		}
	}
	if src := str(r.Data, "source"); src != "" && src != "manual" && src != "sweep" && src != "acceptance" {
		return ErrInvalidUpload
	}
	if stage := str(r.Data, "stage"); stage != "" && !validStage(stage) {
		return ErrInvalidUpload
	}
	return nil
}
func validStage(s string) bool {
	return s == "transcription" || s == "reasoning" || s == "loading" || s == "warmup"
}
func validStatus(s, kind string) bool {
	switch s {
	case "planned", "running", "succeeded", "failed", "cancelled", "interrupted", "skipped":
		return true
	case "active":
		return kind == "conversation"
	}
	return false
}
func terminal(s string) bool {
	return s == "succeeded" || s == "failed" || s == "cancelled" || s == "interrupted" || s == "skipped"
}
func str(m map[string]json.RawMessage, k string) string {
	var s string
	_ = json.Unmarshal(m[k], &s)
	return s
}
func number(m map[string]json.RawMessage, k string) *int64 {
	var n int64
	if json.Unmarshal(m[k], &n) != nil || string(m[k]) == "null" {
		return nil
	}
	return &n
}
func containsContent(m map[string]json.RawMessage) bool {
	for k, v := range m {
		switch strings.ToLower(k) {
		case "transcript", "transcript_text", "reply", "reply_text", "audio", "audio_base64", "error", "error_message", "prompt", "messages":
			return true
		}
		var child map[string]json.RawMessage
		if json.Unmarshal(v, &child) == nil && child != nil && containsContent(child) {
			return true
		}
	}
	return false
}
