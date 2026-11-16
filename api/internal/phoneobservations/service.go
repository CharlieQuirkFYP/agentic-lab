package phoneobservations

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/url"
	"sort"
	"strconv"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

type Query struct {
	InstallationID  string `json:"installation_id,omitempty"`
	ConversationID  string `json:"conversation_id,omitempty"`
	SweepID         string `json:"sweep_id,omitempty"`
	ConfigurationID string `json:"configuration_id,omitempty"`
	SinceMS         *int64 `json:"since_ms,omitempty"`
	UntilMS         *int64 `json:"until_ms,omitempty"`
	Source          string `json:"source,omitempty"`
	Stage           string `json:"stage,omitempty"`
	Outcome         string `json:"outcome,omitempty"`
	RunID           string `json:"run_id,omitempty"`
	AsOf            string `json:"-"`
	Cursor          string `json:"-"`
	Limit           int    `json:"-"`
}

func ParseQuery(v url.Values, paginated, legacy bool) (Query, error) {
	q := Query{Limit: 100}
	for k, values := range v {
		if len(values) != 1 || values[0] == "" {
			return q, ErrInvalidQuery
		}
		value := values[0]
		switch k {
		case "installation_id":
			q.InstallationID = value
		case "conversation_id":
			q.ConversationID = value
		case "sweep_id":
			q.SweepID = value
		case "configuration_id":
			q.ConfigurationID = value
		case "source":
			q.Source = value
		case "stage":
			q.Stage = value
		case "outcome":
			q.Outcome = value
		case "run_id":
			if !legacy {
				return q, ErrInvalidQuery
			}
			q.RunID = value
		case "as_of":
			q.AsOf = value
		case "cursor":
			if !paginated {
				return q, ErrInvalidQuery
			}
			q.Cursor = value
		case "limit":
			if !paginated {
				return q, ErrInvalidQuery
			}
			n, err := strconv.Atoi(value)
			if err != nil || n < 1 {
				return q, ErrInvalidQuery
			}
			q.Limit = n
		case "since_ms", "until_ms":
			n, err := strconv.ParseInt(value, 10, 64)
			if err != nil || n < 0 {
				return q, ErrInvalidQuery
			}
			if k == "since_ms" {
				q.SinceMS = &n
			} else {
				q.UntilMS = &n
			}
		default:
			return q, ErrInvalidQuery
		}
	}
	return q, q.Validate()
}
func (q Query) Validate() error {
	if q.Limit < 0 || q.Limit > 100 || q.SinceMS != nil && q.UntilMS != nil && *q.SinceMS > *q.UntilMS {
		return ErrInvalidQuery
	}
	for _, s := range []string{q.InstallationID, q.ConversationID, q.SweepID, q.ConfigurationID, q.RunID} {
		if s != "" && !validID(s) {
			return ErrInvalidQuery
		}
	}
	if q.Source != "" && q.Source != "manual" && q.Source != "sweep" && q.Source != "acceptance" || q.Stage != "" && !validStage(q.Stage) || q.Outcome != "" && !validStatus(q.Outcome, "conversation") {
		return ErrInvalidQuery
	}
	return nil
}

type Service struct{ store *Store }

func NewService(store *Store) *Service { return &Service{store: store} }
func (s *Service) Append(ctx context.Context, u Upload) (Accepted, error) {
	return s.store.Append(ctx, u)
}

type cursor struct {
	AsOf   string `json:"a"`
	Offset int    `json:"o"`
	Filter string `json:"f"`
	Kind   string `json:"k"`
}

func (q Query) fingerprint() string {
	b, _ := json.Marshal(q)
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}
func prepare(q Query, kind string) (Query, int, error) {
	if err := q.Validate(); err != nil {
		return q, 0, err
	}
	if q.Limit == 0 {
		q.Limit = 100
	}
	if q.Cursor == "" {
		return q, 0, nil
	}
	var c cursor
	if decode(q.Cursor, &c) != nil || c.Offset < 0 || c.Filter != q.fingerprint() || c.Kind != kind || q.AsOf != "" && q.AsOf != c.AsOf {
		return q, 0, ErrInvalidQuery
	}
	q.AsOf = c.AsOf
	return q, c.Offset, nil
}
func next(q Query, kind, asOf string, offset, total int) *string {
	if offset >= total {
		return nil
	}
	s := encode(cursor{asOf, offset, q.fingerprint(), kind})
	return &s
}
func (s *Service) List(ctx context.Context, q Query) (Page, error) {
	q, offset, err := prepare(q, "records")
	if err != nil {
		return Page{}, err
	}
	snap, err := s.store.snapshot(ctx, q.AsOf)
	if err != nil {
		return Page{}, err
	}
	selected := selectSnapshot(snap, q)
	p := Page{SchemaVersion: 1, AsOf: snap.asOf, Records: []StoredRecord{}, Total: len(selected.records), SelectedRecordKeys: []RecordKey{}, Filtered: q.filtered()}
	if offset < p.Total {
		end := min(offset+q.Limit, p.Total)
		p.Records = selected.records[offset:end]
		for _, r := range p.Records {
			k := RecordKey{r.Device.InstallationID, r.Record.RecordID}
			if selected.selected[k] {
				p.SelectedRecordKeys = append(p.SelectedRecordKeys, k)
			}
		}
		p.NextCursor = next(q, "records", p.AsOf, end, p.Total)
	}
	return p, nil
}
func (s *Service) ListLegacy(ctx context.Context, q Query) (LegacyPage, error) {
	q, offset, err := prepare(q, "legacy")
	if err != nil {
		return LegacyPage{}, err
	}
	snap, err := s.store.snapshot(ctx, q.AsOf)
	if err != nil {
		return LegacyPage{}, err
	}
	selected := selectSnapshot(snap, q)
	p := LegacyPage{SchemaVersion: 1, AsOf: snap.asOf, Batches: []LegacyRecord{}, Total: len(selected.legacy)}
	if offset < p.Total {
		end := min(offset+q.Limit, p.Total)
		p.Batches = selected.legacy[offset:end]
		p.NextCursor = next(q, "legacy", p.AsOf, end, p.Total)
	}
	return p, nil
}
func (s *Service) Overview(ctx context.Context, q Query) (Overview, error) {
	if err := q.Validate(); err != nil {
		return Overview{}, err
	}
	snap, err := s.store.snapshot(ctx, q.AsOf)
	if err != nil {
		return Overview{}, err
	}
	return summarize(selectSnapshot(snap, q)), nil
}

type entityKey struct{ installation, kind, id string }

func key(r StoredRecord) entityKey {
	return entityKey{r.Device.InstallationID, r.Record.Kind, r.Record.EntityID}
}
func latest(records []StoredRecord) map[entityKey]StoredRecord {
	m := map[entityKey]StoredRecord{}
	for _, r := range records {
		k := key(r)
		old, ok := m[k]
		if !ok || r.Record.Revision > old.Record.Revision {
			m[k] = r
		}
	}
	return m
}

type attribution struct {
	context                                   Context
	source, stage, outcome, dataset, protocol string
	configuration                             map[string]json.RawMessage
	unresolved                                bool
}

func ownContext(r Observation) Context {
	c := r.Context
	switch r.Kind {
	case "configuration":
		c.ConfigurationID = r.EntityID
	case "conversation":
		c.ConversationID = r.EntityID
	case "turn":
		c.TurnID = r.EntityID
	case "attempt":
		c.AttemptID = r.EntityID
	case "sweep":
		c.SweepID = r.EntityID
	case "job":
		c.JobID = r.EntityID
	}
	return c
}
func refs(c Context) map[string]string {
	return map[string]string{"configuration": c.ConfigurationID, "conversation": c.ConversationID, "turn": c.TurnID, "attempt": c.AttemptID, "sweep": c.SweepID, "job": c.JobID}
}
func mergeContext(c *Context, p Context) {
	if c.ConfigurationID == "" {
		c.ConfigurationID = p.ConfigurationID
	}
	if c.ConversationID == "" {
		c.ConversationID = p.ConversationID
	}
	if c.TurnID == "" {
		c.TurnID = p.TurnID
	}
	if c.AttemptID == "" {
		c.AttemptID = p.AttemptID
	}
	if c.SweepID == "" {
		c.SweepID = p.SweepID
	}
	if c.JobID == "" {
		c.JobID = p.JobID
	}
	if c.RuntimeSessionID == "" {
		c.RuntimeSessionID = p.RuntimeSessionID
	}
}
func resolve(r StoredRecord, index map[entityKey]StoredRecord) attribution {
	a := attribution{context: ownContext(r.Record), source: str(r.Record.Data, "source"), stage: str(r.Record.Data, "stage"), outcome: str(r.Record.Data, "status"), dataset: str(r.Record.Data, "dataset_revision"), protocol: str(r.Record.Data, "protocol_revision")}
	if r.Record.Kind == "metric_batch" || r.Record.Kind == "finalization" {
		a.outcome = ""
	}
	visited := map[entityKey]bool{key(r): true}
	for range 8 {
		changed := false
		ref := refs(a.context)
		for _, kind := range []string{"attempt", "turn", "conversation", "job", "sweep", "configuration"} {
			id := ref[kind]
			if id == "" {
				continue
			}
			k := entityKey{r.Device.InstallationID, kind, id}
			p, ok := index[k]
			if !ok {
				a.unresolved = true
				continue
			}
			if kind == "configuration" {
				a.configuration = p.Record.Data
			}
			if visited[k] {
				continue
			}
			visited[k] = true
			changed = true
			mergeContext(&a.context, ownContext(p.Record))
			if a.source == "" {
				a.source = str(p.Record.Data, "source")
			}
			if a.stage == "" {
				a.stage = str(p.Record.Data, "stage")
			}
			if a.outcome == "" && kind != "configuration" {
				a.outcome = str(p.Record.Data, "status")
			}
			if a.dataset == "" {
				a.dataset = str(p.Record.Data, "dataset_revision")
			}
			if a.protocol == "" {
				a.protocol = str(p.Record.Data, "protocol_revision")
			}
		}
		if !changed {
			break
		}
	}
	return a
}
func (q Query) matches(r StoredRecord, a attribution) bool {
	return (q.InstallationID == "" || q.InstallationID == r.Device.InstallationID) && (q.ConversationID == "" || q.ConversationID == a.context.ConversationID) && (q.SweepID == "" || q.SweepID == a.context.SweepID) && (q.ConfigurationID == "" || q.ConfigurationID == a.context.ConfigurationID) && (q.Source == "" || q.Source == a.source) && (q.Stage == "" || q.Stage == a.stage) && (q.Outcome == "" || q.Outcome == a.outcome) && (q.SinceMS == nil || r.Record.TimestampMS >= *q.SinceMS) && (q.UntilMS == nil || r.Record.TimestampMS <= *q.UntilMS)
}
func selectSnapshot(s snapshot, q Query) snapshot {
	index := latest(s.records)
	selected := map[entityKey]bool{}
	selectedRecords := map[RecordKey]bool{}
	for _, r := range s.records {
		current := index[key(r)]
		a := resolve(current, index)
		if q.matches(r, a) {
			selected[key(r)] = true
			selectedRecords[RecordKey{r.Device.InstallationID, r.Record.RecordID}] = true
		}
	}
	// Add only referenced ancestors, then retain every revision of selected entities.
	for range 8 {
		changed := false
		for k := range selected {
			r, ok := index[k]
			if !ok {
				continue
			}
			a := resolve(r, index)
			for kind, id := range refs(a.context) {
				if id == "" {
					continue
				}
				parent := entityKey{k.installation, kind, id}
				if _, ok := index[parent]; ok && !selected[parent] {
					selected[parent] = true
					changed = true
				}
			}
		}
		if !changed {
			break
		}
	}
	out := snapshot{asOf: s.asOf, records: []StoredRecord{}, legacy: []LegacyRecord{}, selected: selectedRecords}
	legacyAttrs := legacyAttributions(s.legacy)
	canonical := map[string]bool{}
	for _, r := range s.records {
		if r.Record.Kind == "metric_batch" {
			var b metrics.Batch
			if json.Unmarshal(r.Record.Data["batch"], &b) == nil {
				canonical[batchIdentity(r.Device.InstallationID, b)] = true
			}
		}
		if selected[key(r)] {
			out.records = append(out.records, r)
		}
	}
	for _, r := range s.legacy {
		installation := ""
		if r.Device != nil {
			installation = r.Device.InstallationID
		}
		if canonical[batchIdentity(installation, r.Batch)] {
			continue
		}
		if q.InstallationID != "" && q.InstallationID != installation || q.ConversationID != "" || q.SweepID != "" || q.ConfigurationID != "" || q.Source != "" || q.Outcome != "" || q.SinceMS != nil && r.Batch.TimestampMS < *q.SinceMS || q.UntilMS != nil && r.Batch.TimestampMS > *q.UntilMS || q.RunID != "" && r.Batch.RunID != q.RunID {
			continue
		}
		if q.Stage != "" && legacyAttrs[legacyKey(r)].stage != q.Stage {
			continue
		}
		out.legacy = append(out.legacy, r)
	}
	return out
}

func (q Query) filtered() bool {
	return q.InstallationID != "" || q.ConversationID != "" || q.ConfigurationID != "" || q.SweepID != "" || q.Source != "" || q.Stage != "" || q.Outcome != "" || q.SinceMS != nil || q.UntilMS != nil || q.RunID != ""
}
func batchIdentity(installation string, b metrics.Batch) string {
	b.ExperimentID = ""
	b.Events = append([]metrics.Event(nil), b.Events...)
	for i := range b.Events {
		b.Events[i].ExperimentID = ""
	}
	raw, _ := json.Marshal(b)
	sum := sha256.Sum256(raw)
	return installation + ":" + hex.EncodeToString(sum[:])
}

func metricBatch(r Observation) (metrics.Batch, bool) {
	var b metrics.Batch
	err := json.Unmarshal(r.Data["batch"], &b)
	return b, err == nil
}
func canonicalBatchIdentity(r StoredRecord, a attribution, b metrics.Batch) string {
	runtimeID := a.context.RuntimeSessionID
	if a.context.AttemptID != "" {
		runtimeID = ""
	}
	return batchIdentity(r.Device.InstallationID, b) + "\x00" + runtimeID + "\x00" + a.context.AttemptID
}
func legacyStage(b metrics.Batch) string {
	for _, e := range b.Events {
		if e.Name == "model_purpose" {
			if s, ok := e.Value.(string); ok && (s == "transcription" || s == "reasoning") {
				return s
			}
		}
	}
	return ""
}
func sortedRecords(m map[entityKey]StoredRecord) []StoredRecord {
	out := make([]StoredRecord, 0, len(m))
	for _, r := range m {
		out = append(out, r)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].id < out[j].id })
	return out
}
