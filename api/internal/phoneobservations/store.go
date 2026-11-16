package phoneobservations

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
)

type Store struct{ db *sql.DB }

func NewStore(db *sql.DB) *Store { return &Store{db: db} }

func (s *Store) Append(ctx context.Context, u Upload) (Accepted, error) {
	result := Accepted{Status: "accepted", RecordCount: len(u.Records), AcceptedRecordIDs: []string{}}
	if err := u.Validate(); err != nil {
		return result, err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return result, err
	}
	defer func() { _ = tx.Rollback() }()
	device, _ := json.Marshal(u.Device)
	now := time.Now().UnixMilli()
	for _, r := range u.Records {
		result.AcceptedRecordIDs = append(result.AcceptedRecordIDs, r.RecordID)
		payload, err := json.Marshal(r)
		if err != nil {
			return result, err
		}
		payload = canonicalJSON(payload)
		var stored, storedDevice string
		err = tx.QueryRowContext(ctx, `SELECT payload,device FROM phone_observations WHERE installation_id=? AND (record_id=? OR (producer_stream_id=? AND record_sequence=?) OR (kind=? AND entity_id=? AND revision=?)) LIMIT 1`, u.Device.InstallationID, r.RecordID, r.ProducerStreamID, r.RecordSequence, r.Kind, r.EntityID, r.Revision).Scan(&stored, &storedDevice)
		if err == nil {
			if stored != string(payload) || storedDevice != string(device) {
				return result, ErrConflict
			}
			continue
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return result, err
		}
		if r.Kind == "metric_batch" {
			b, _ := metricBatch(r)
			b.ExperimentID = ""
			b.Events = append([]metrics.Event(nil), b.Events...)
			for i := range b.Events {
				b.Events[i].ExperimentID = ""
			}
			raw, _ := json.Marshal(b)
			raw = canonicalJSON(raw)
			runtimeID := r.Context.RuntimeSessionID
			if r.Context.AttemptID != "" {
				runtimeID = ""
			}
			_, err = tx.ExecContext(ctx, `INSERT INTO phone_observation_metric_identities(installation_id,attempt_id,runtime_session_id,run_id,sequence_number,payload) VALUES(?,?,?,?,?,?) ON CONFLICT DO NOTHING`, u.Device.InstallationID, r.Context.AttemptID, runtimeID, b.RunID, b.SequenceNumber, string(raw))
			if err != nil {
				return result, err
			}
			var existing string
			if err = tx.QueryRowContext(ctx, `SELECT payload FROM phone_observation_metric_identities WHERE installation_id=? AND attempt_id=? AND runtime_session_id=? AND run_id=? AND sequence_number=?`, u.Device.InstallationID, r.Context.AttemptID, runtimeID, b.RunID, b.SequenceNumber).Scan(&existing); err != nil {
				return result, err
			}
			if existing != string(raw) {
				return result, ErrConflict
			}
		}
		// Configuration IDs are immutable; terminal lifecycle entities cannot reopen.
		rows, err := tx.QueryContext(ctx, `SELECT payload FROM phone_observations WHERE installation_id=? AND kind=? AND entity_id=?`, u.Device.InstallationID, r.Kind, r.EntityID)
		if err != nil {
			return result, err
		}
		for rows.Next() {
			var raw string
			if err = rows.Scan(&raw); err != nil {
				break
			}
			var prior Observation
			if err = json.Unmarshal([]byte(raw), &prior); err != nil {
				break
			}
			a, b := str(prior.Data, "status"), str(r.Data, "status")
			if r.Kind == "configuration" {
				aJSON, _ := json.Marshal(prior.Data)
				bJSON, _ := json.Marshal(r.Data)
				if string(aJSON) != string(bJSON) {
					err = ErrConflict
					break
				}
			}
			if terminal(a) && r.Revision > prior.Revision && b != a || terminal(b) && prior.Revision > r.Revision && a != b {
				err = ErrConflict
				break
			}
		}
		rowErr := rows.Err()
		_ = rows.Close()
		if err != nil {
			return result, err
		}
		if rowErr != nil {
			return result, rowErr
		}
		_, err = tx.ExecContext(ctx, `INSERT INTO phone_observations(installation_id,record_id,producer_stream_id,record_sequence,kind,entity_id,revision,timestamp_ms,conversation_id,sweep_id,configuration_id,received_at_ms,device,payload) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)`, u.Device.InstallationID, r.RecordID, r.ProducerStreamID, r.RecordSequence, r.Kind, r.EntityID, r.Revision, r.TimestampMS, r.Context.ConversationID, r.Context.SweepID, r.Context.ConfigurationID, now, string(device), string(payload))
		if err != nil {
			return result, err
		}
	}
	return result, tx.Commit()
}

func canonicalJSON(raw []byte) []byte {
	var v any
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if d.Decode(&v) != nil {
		return raw
	}
	out, err := json.Marshal(v)
	if err != nil {
		return raw
	}
	return out
}

type watermark struct {
	Version int   `json:"v"`
	Record  int64 `json:"r"`
	Legacy  int64 `json:"l"`
	Time    int64 `json:"t"`
}

func encode(v any) string { b, _ := json.Marshal(v); return base64.RawURLEncoding.EncodeToString(b) }
func decode(s string, v any) error {
	b, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil || json.Unmarshal(b, v) != nil {
		return ErrInvalidQuery
	}
	return nil
}

type snapshot struct {
	asOf     string
	records  []StoredRecord
	legacy   []LegacyRecord
	selected map[RecordKey]bool
}

func (s *Store) snapshot(ctx context.Context, asOf string) (snapshot, error) {
	out := snapshot{records: []StoredRecord{}, legacy: []LegacyRecord{}}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer func() { _ = tx.Rollback() }()
	var w watermark
	if asOf == "" {
		w.Version = 1
		w.Time = time.Now().UnixMilli()
		if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(id),0) FROM phone_observations`).Scan(&w.Record); err != nil {
			return out, err
		}
		if err := tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(rowid),0) FROM metric_batches`).Scan(&w.Legacy); err != nil {
			return out, err
		}
		asOf = encode(w)
	} else if decode(asOf, &w) != nil || w.Version != 1 || w.Record < 0 || w.Legacy < 0 || w.Time < 0 {
		return out, ErrInvalidQuery
	}
	out.asOf = asOf
	rows, err := tx.QueryContext(ctx, `SELECT id,device,payload,received_at_ms FROM phone_observations WHERE id<=? ORDER BY id`, w.Record)
	if err != nil {
		return out, err
	}
	for rows.Next() {
		var r StoredRecord
		var device, payload string
		if err = rows.Scan(&r.id, &device, &payload, &r.ReceivedAtMS); err != nil {
			break
		}
		if err = json.Unmarshal([]byte(device), &r.Device); err != nil {
			break
		}
		if err = json.Unmarshal([]byte(payload), &r.Record); err != nil {
			break
		}
		out.records = append(out.records, r)
	}
	rowErr := rows.Err()
	_ = rows.Close()
	if err != nil {
		return out, err
	}
	if rowErr != nil {
		return out, rowErr
	}
	rows, err = tx.QueryContext(ctx, `SELECT m.rowid,m.payload,e.payload,e.created_at FROM metric_batches m JOIN experiments e ON m.experiment_id=e.id WHERE m.rowid<=? AND json_extract(e.payload,'$.result_source')=? ORDER BY m.rowid`, w.Legacy, phonemetrics.Source)
	if err != nil {
		return out, err
	}
	for rows.Next() {
		var r LegacyRecord
		var payload, experiment, created string
		if err = rows.Scan(&r.id, &payload, &experiment, &created); err != nil {
			break
		}
		if err = json.Unmarshal([]byte(payload), &r.Batch); err != nil {
			break
		}
		var exp benchmark.Experiment
		if err = json.Unmarshal([]byte(experiment), &exp); err != nil {
			break
		}
		if exp.Device != nil {
			r.Device = &phonemetrics.Device{InstallationID: exp.Device.InstallationID, Hardware: exp.Device.Hardware, OSVersion: exp.Device.OSVersion, AppVersion: exp.Device.AppVersion, Build: exp.Device.Build}
		}
		t, _ := time.Parse(time.RFC3339Nano, created)
		if !t.IsZero() {
			r.ReceivedAtMS = t.UnixMilli()
		}
		out.legacy = append(out.legacy, r)
	}
	rowErr = rows.Err()
	_ = rows.Close()
	if err != nil {
		return out, err
	}
	if rowErr != nil {
		return out, rowErr
	}
	return out, tx.Commit()
}
