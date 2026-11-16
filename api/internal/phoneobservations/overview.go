package phoneobservations

import (
	"encoding/json"
	"sort"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
)

type OutcomeCounts struct {
	Total    int            `json:"total"`
	Outcomes map[string]int `json:"outcomes"`
}
type Counts struct {
	Configurations int           `json:"configurations"`
	Conversations  OutcomeCounts `json:"conversations"`
	Turns          OutcomeCounts `json:"turns"`
	Attempts       OutcomeCounts `json:"attempts"`
	Sweeps         OutcomeCounts `json:"sweeps"`
	Jobs           OutcomeCounts `json:"jobs"`
}
type FinalizationCoverage struct {
	Complete      int   `json:"complete"`
	Partial       int   `json:"partial"`
	Unknown       int   `json:"unknown"`
	DroppedEvents int64 `json:"dropped_events"`
}
type Coverage struct {
	RecordCount             int                  `json:"record_count"`
	MetricBatchCount        int                  `json:"metric_batch_count"`
	EventCount              int                  `json:"event_count"`
	LegacyBatchCount        int                  `json:"legacy_batch_count"`
	LegacyConversationCount *int                 `json:"legacy_conversation_count"`
	FirstTimestampMS        *int64               `json:"first_timestamp_ms"`
	LastTimestampMS         *int64               `json:"last_timestamp_ms"`
	LastReceivedAtMS        *int64               `json:"last_received_at_ms"`
	UnresolvedRecordCount   int                  `json:"unresolved_record_count"`
	Finalization            FinalizationCoverage `json:"finalization"`
	RetainedHistory         bool                 `json:"retained_history"`
}
type MetricSeries struct {
	InstallationID   *string                    `json:"installation_id"`
	Device           *phonemetrics.Device       `json:"device"`
	ConfigurationID  *string                    `json:"configuration_id"`
	Configuration    map[string]json.RawMessage `json:"configuration"`
	Stage            *string                    `json:"stage"`
	DatasetRevision  *string                    `json:"dataset_revision"`
	ProtocolRevision *string                    `json:"protocol_revision"`
	Outcome          *string                    `json:"outcome"`
	benchmark.MetricSummary
	Latest            any            `json:"latest"`
	LatestTimestampMS int64          `json:"latest_timestamp_ms"`
	StatusCounts      map[string]int `json:"status_counts"`
}
type Activity struct {
	InstallationID   *string                    `json:"installation_id"`
	Device           *phonemetrics.Device       `json:"device"`
	EntityID         string                     `json:"entity_id"`
	Kind             string                     `json:"kind"`
	Status           *string                    `json:"status"`
	TimestampMS      int64                      `json:"timestamp_ms"`
	StartedAtMS      *int64                     `json:"started_at_ms"`
	FinishedAtMS     *int64                     `json:"finished_at_ms"`
	ConfigurationID  *string                    `json:"configuration_id"`
	Configuration    map[string]json.RawMessage `json:"configuration"`
	Context          map[string]string          `json:"context"`
	Source           *string                    `json:"source"`
	Stage            *string                    `json:"stage"`
	MetricBatchCount int                        `json:"metric_batch_count"`
	EventCount       int                        `json:"event_count"`
	Completeness     string                     `json:"completeness"`
	Metrics          []MetricSeries             `json:"metrics"`
}
type Overview struct {
	SchemaVersion       int            `json:"schema_version"`
	AsOf                string         `json:"as_of"`
	Source              string         `json:"source"`
	Coverage            Coverage       `json:"coverage"`
	Counts              Counts         `json:"counts"`
	Series              []MetricSeries `json:"series"`
	CurrentConversation *Activity      `json:"current_conversation"`
	RecentConversations []Activity     `json:"recent_conversations"`
	RecentRuns          []Activity     `json:"recent_runs"`
	RecentSweeps        []Activity     `json:"recent_sweeps"`
}
type observedBatch struct {
	device    *phonemetrics.Device
	batch     metrics.Batch
	attr      attribution
	legacyRun string
	identity  string
}

func ptr(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}
func outcomeCounts() OutcomeCounts { return OutcomeCounts{Outcomes: map[string]int{}} }
func summarize(s snapshot) Overview {
	out := Overview{SchemaVersion: 1, AsOf: s.asOf, Source: phonemetrics.Source, Coverage: Coverage{RecordCount: len(s.records), LegacyBatchCount: len(s.legacy), RetainedHistory: true}, Counts: Counts{Conversations: outcomeCounts(), Turns: outcomeCounts(), Attempts: outcomeCounts(), Sweeps: outcomeCounts(), Jobs: outcomeCounts()}, Series: []MetricSeries{}, RecentConversations: []Activity{}, RecentRuns: []Activity{}, RecentSweeps: []Activity{}}
	index := latest(s.records)
	records := sortedRecords(index)
	batches := []observedBatch{}
	dedup := map[string]bool{}
	finals := map[entityKey]StoredRecord{}
	for _, r := range s.records {
		touchCoverage(&out.Coverage, r.Record.TimestampMS, r.ReceivedAtMS)
		if resolve(r, index).unresolved {
			out.Coverage.UnresolvedRecordCount++
		}
	}
	for _, r := range records {
		a := resolve(r, index)
		switch r.Record.Kind {
		case "configuration":
			out.Counts.Configurations++
		case "conversation":
			countOutcome(&out.Counts.Conversations, str(r.Record.Data, "status"))
		case "turn":
			countOutcome(&out.Counts.Turns, str(r.Record.Data, "status"))
		case "attempt":
			countOutcome(&out.Counts.Attempts, str(r.Record.Data, "status"))
		case "sweep":
			countOutcome(&out.Counts.Sweeps, str(r.Record.Data, "status"))
		case "job":
			countOutcome(&out.Counts.Jobs, str(r.Record.Data, "status"))
		case "metric_batch":
			if b, ok := metricBatch(r.Record); ok {
				id := canonicalBatchIdentity(r, a, b)
				if !dedup[id] {
					dedup[id] = true
					batches = append(batches, observedBatch{device: &r.Device, batch: b, attr: a, identity: id})
				}
			}
		case "finalization":
			k := finalizationTarget(r)
			old, ok := finals[k]
			if !ok || r.Record.TimestampMS > old.Record.TimestampMS || r.Record.TimestampMS == old.Record.TimestampMS && r.id > old.id {
				finals[k] = r
			}
		}
	}
	completenessStates := completeness(index, s.records, finals)
	for k := range finals {
		if completenessStates[k] == "complete" {
			out.Coverage.Finalization.Complete++
		} else {
			out.Coverage.Finalization.Partial++
		}
	}
	out.Coverage.Finalization.DroppedEvents = droppedEventTotal(finals, index)
	for _, r := range records {
		if r.Record.Kind == "conversation" || r.Record.Kind == "attempt" || r.Record.Kind == "sweep" {
			if _, ok := finals[key(r)]; !ok {
				out.Coverage.Finalization.Unknown++
			}
		}
	}
	legacyAttrs := legacyAttributions(s.legacy)
	for _, r := range s.legacy {
		touchCoverage(&out.Coverage, r.Batch.TimestampMS, r.ReceivedAtMS)
		installation := ""
		if r.Device != nil {
			installation = r.Device.InstallationID
		}
		id := batchIdentity(installation, r.Batch)
		if dedup[id] {
			continue
		}
		dedup[id] = true
		batches = append(batches, observedBatch{device: r.Device, batch: r.Batch, attr: legacyAttrs[legacyKey(r)], legacyRun: r.Batch.RunID, identity: id})
	}
	out.Coverage.MetricBatchCount = len(batches)
	for _, b := range batches {
		out.Coverage.EventCount += len(b.batch.Events)
	}
	out.Series = summarizeSeries(batches)
	for _, r := range records {
		if r.Record.Kind != "conversation" && r.Record.Kind != "attempt" && r.Record.Kind != "sweep" {
			continue
		}
		a := resolve(r, index)
		activity := Activity{InstallationID: ptr(r.Device.InstallationID), Device: &r.Device, EntityID: r.Record.EntityID, Kind: r.Record.Kind, Status: ptr(str(r.Record.Data, "status")), TimestampMS: r.Record.TimestampMS, StartedAtMS: number(r.Record.Data, "started_at_ms"), FinishedAtMS: number(r.Record.Data, "finished_at_ms"), ConfigurationID: ptr(a.context.ConfigurationID), Configuration: a.configuration, Context: contextMap(a.context), Source: ptr(a.source), Stage: ptr(a.stage), Completeness: "unknown", Metrics: []MetricSeries{}}
		if final, ok := finals[key(r)]; ok {
			_ = final
			activity.Completeness = completenessStates[key(r)]
		}
		if a.unresolved && activity.Completeness == "complete" {
			activity.Completeness = "partial"
		}
		owned := []observedBatch{}
		for _, b := range batches {
			if b.device == nil || b.device.InstallationID != r.Device.InstallationID {
				continue
			}
			id := ""
			switch r.Record.Kind {
			case "conversation":
				id = b.attr.context.ConversationID
			case "attempt":
				id = b.attr.context.AttemptID
			case "sweep":
				id = b.attr.context.SweepID
			}
			if id == r.Record.EntityID {
				owned = append(owned, b)
			}
		}
		activity.MetricBatchCount = len(owned)
		for _, b := range owned {
			activity.EventCount += len(b.batch.Events)
		}
		activity.Metrics = summarizeSeries(owned)
		switch r.Record.Kind {
		case "conversation":
			out.RecentConversations = append(out.RecentConversations, activity)
		case "attempt":
			out.RecentRuns = append(out.RecentRuns, activity)
		case "sweep":
			out.RecentSweeps = append(out.RecentSweeps, activity)
		}
	}
	legacyRuns := map[string][]observedBatch{}
	for _, b := range batches {
		if b.legacyRun != "" {
			installation := ""
			if b.device != nil {
				installation = b.device.InstallationID
			}
			k := installation + "\x00" + b.batch.ExperimentID + "\x00" + b.legacyRun
			legacyRuns[k] = append(legacyRuns[k], b)
		}
	}
	for _, group := range legacyRuns {
		b := group[0]
		installation := ""
		if b.device != nil {
			installation = b.device.InstallationID
		}
		activity := Activity{InstallationID: ptr(installation), Device: b.device, EntityID: b.legacyRun, Kind: "legacy_run", Configuration: b.attr.configuration, Context: map[string]string{}, Stage: ptr(b.attr.stage), MetricBatchCount: len(group), Completeness: "unknown", Metrics: summarizeSeries(group)}
		for _, v := range group {
			activity.TimestampMS = max(activity.TimestampMS, v.batch.TimestampMS)
			activity.EventCount += len(v.batch.Events)
			for _, e := range v.batch.Events {
				if e.Name == "native_run_id" || e.Name == "turn_id" {
					if value, ok := e.Value.(string); ok {
						activity.Context[e.Name] = value
					}
				}
			}
		}
		out.RecentRuns = append(out.RecentRuns, activity)
	}
	sortActivities(out.RecentConversations)
	sortActivities(out.RecentRuns)
	sortActivities(out.RecentSweeps)
	for _, r := range out.RecentConversations {
		if r.Status != nil && (*r.Status == "active" || *r.Status == "running") {
			out.CurrentConversation = &r
			break
		}
	}
	if out.CurrentConversation == nil && len(out.RecentConversations) > 0 {
		out.CurrentConversation = &out.RecentConversations[0]
	}
	out.RecentConversations = out.RecentConversations[:min(20, len(out.RecentConversations))]
	out.RecentRuns = out.RecentRuns[:min(20, len(out.RecentRuns))]
	out.RecentSweeps = out.RecentSweeps[:min(20, len(out.RecentSweeps))]
	return out
}
func countOutcome(c *OutcomeCounts, s string) { c.Total++; c.Outcomes[s]++ }
func touchCoverage(c *Coverage, t, receipt int64) {
	if c.FirstTimestampMS == nil || t < *c.FirstTimestampMS {
		v := t
		c.FirstTimestampMS = &v
	}
	if c.LastTimestampMS == nil || t > *c.LastTimestampMS {
		v := t
		c.LastTimestampMS = &v
	}
	if receipt > 0 && (c.LastReceivedAtMS == nil || receipt > *c.LastReceivedAtMS) {
		v := receipt
		c.LastReceivedAtMS = &v
	}
}
func contextMap(c Context) map[string]string {
	b, _ := json.Marshal(c)
	m := map[string]string{}
	_ = json.Unmarshal(b, &m)
	return m
}
func finalizationTarget(r StoredRecord) entityKey {
	c := r.Record.Context
	kind, id := "sweep", c.SweepID
	if c.ConversationID != "" {
		kind, id = "conversation", c.ConversationID
	}
	if c.AttemptID != "" {
		kind, id = "attempt", c.AttemptID
	}
	return entityKey{r.Device.InstallationID, kind, id}
}

// Parent finalizations repeat aggregate drop counts. Count each subtree once,
// retaining a parent's larger total when some descendant evidence is absent.
func droppedEventTotal(finals, index map[entityKey]StoredRecord) int64 {
	type node struct {
		own      int64
		parent   entityKey
		children map[entityKey]bool
	}
	nodes := map[entityKey]*node{}
	get := func(k entityKey) *node {
		if nodes[k] == nil {
			nodes[k] = &node{children: map[entityKey]bool{}}
		}
		return nodes[k]
	}
	for _, r := range finals {
		c := resolve(r, index).context
		chain := []entityKey{}
		for _, entry := range []struct{ kind, id string }{{"sweep", c.SweepID}, {"conversation", c.ConversationID}, {"attempt", c.AttemptID}} {
			if entry.id != "" {
				chain = append(chain, entityKey{r.Device.InstallationID, entry.kind, entry.id})
			}
		}
		if len(chain) == 0 {
			chain = append(chain, key(r))
		}
		for i, k := range chain {
			n := get(k)
			if i > 0 {
				n.parent = chain[i-1]
				get(n.parent).children[k] = true
			}
		}
		if value := number(r.Record.Data, "dropped_events"); value != nil {
			get(chain[len(chain)-1]).own = *value
		}
	}
	var subtotal func(entityKey) int64
	subtotal = func(k entityKey) int64 {
		n := nodes[k]
		var children int64
		for child := range n.children {
			children += subtotal(child)
		}
		if children > n.own {
			return children
		}
		return n.own
	}
	var total int64
	for k, n := range nodes {
		if n.parent.kind == "" {
			total += subtotal(k)
		}
	}
	return total
}
func sortActivities(a []Activity) {
	sort.Slice(a, func(i, j int) bool {
		if a[i].TimestampMS != a[j].TimestampMS {
			return a[i].TimestampMS > a[j].TimestampMS
		}
		if a[i].EntityID != a[j].EntityID {
			return a[i].EntityID < a[j].EntityID
		}
		return encode(a[i].Device) < encode(a[j].Device)
	})
}
func summarizeSeries(batches []observedBatch) []MetricSeries {
	type group struct {
		series MetricSeries
		events []metrics.Event
	}
	groups := map[string]*group{}
	for _, b := range batches {
		for _, e := range b.batch.Events {
			installation := ""
			if b.device != nil {
				installation = b.device.InstallationID
			}
			a := b.attr
			series := MetricSeries{InstallationID: ptr(installation), Device: b.device, ConfigurationID: ptr(a.context.ConfigurationID), Configuration: a.configuration, Stage: ptr(a.stage), DatasetRevision: ptr(a.dataset), ProtocolRevision: ptr(a.protocol), Outcome: ptr(a.outcome), MetricSummary: benchmark.MetricSummary{Name: e.Name, Unit: e.Unit, Scope: e.Scope, Source: e.Source}, StatusCounts: map[string]int{}}
			k := encode(series)
			g := groups[k]
			if g == nil {
				g = &group{series: series}
				groups[k] = g
			}
			g.events = append(g.events, e)
			if len(g.events) == 1 || e.TimestampMS >= g.series.LatestTimestampMS {
				g.series.Latest = e.Value
				g.series.LatestTimestampMS = e.TimestampMS
			}
			switch v := e.Value.(type) {
			case string:
				g.series.StatusCounts[v]++
			case bool:
				if v {
					g.series.StatusCounts["true"]++
				} else {
					g.series.StatusCounts["false"]++
				}
			}
		}
	}
	result := []MetricSeries{}
	keys := make([]string, 0, len(groups))
	for k := range groups {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		g := groups[k]
		summary := benchmark.SummarizeMetrics([]metrics.Batch{{Events: g.events}})
		g.series.MetricSummary = summary[0]
		result = append(result, g.series)
	}
	return result
}
func legacyKey(r LegacyRecord) string {
	installation := ""
	if r.Device != nil {
		installation = r.Device.InstallationID
	}
	return installation + "\x00" + r.Batch.ExperimentID + "\x00" + r.Batch.RunID
}
func legacyAttributions(records []LegacyRecord) map[string]attribution {
	attrs := map[string]attribution{}
	native := map[string]attribution{}
	ambiguous := map[string]bool{}
	for _, r := range records {
		a := attribution{stage: legacyStage(r.Batch)}
		values := map[string]string{}
		for _, e := range r.Batch.Events {
			if v, ok := e.Value.(string); ok {
				values[e.Name] = v
			}
		}
		if values["model_name"] != "" || values["model_sha256"] != "" {
			model, _ := json.Marshal(map[string]string{"name": values["model_name"], "sha256": values["model_sha256"]})
			a.configuration = map[string]json.RawMessage{"asr": json.RawMessage("null"), "reasoning": json.RawMessage("null")}
			if a.stage == "transcription" {
				a.configuration["asr"] = model
			} else if a.stage == "reasoning" {
				a.configuration["reasoning"] = model
			}
		}
		attrs[legacyKey(r)] = a
		if r.Device != nil && values["native_run_id"] != "" && a.stage != "" {
			k := r.Device.InstallationID + "\x00" + values["native_run_id"]
			if old, ok := native[k]; ok && (encode(old.configuration) != encode(a.configuration) || old.stage != a.stage) {
				ambiguous[k] = true
			}
			native[k] = a
		}
	}
	for _, r := range records {
		a := attrs[legacyKey(r)]
		if a.stage == "" && r.Device != nil {
			k := r.Device.InstallationID + "\x00" + r.Batch.RunID
			if found, ok := native[k]; ok && !ambiguous[k] {
				attrs[legacyKey(r)] = found
			}
		}
	}
	return attrs
}
