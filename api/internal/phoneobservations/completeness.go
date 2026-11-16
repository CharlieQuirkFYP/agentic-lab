package phoneobservations

import "encoding/json"

// Completeness is evidence about a specific lifecycle revision, independent of
// whether inference succeeded. A producer's label alone cannot certify receipt.
func completeness(index map[entityKey]StoredRecord, records []StoredRecord, finals map[entityKey]StoredRecord) map[entityKey]string {
	states := map[entityKey]string{}
	byID := map[RecordKey]StoredRecord{}
	for _, r := range records {
		byID[RecordKey{r.Device.InstallationID, r.Record.RecordID}] = r
	}
	var check func(entityKey) string
	check = func(k entityKey) string {
		if state, ok := states[k]; ok {
			return state
		}
		states[k] = "partial" // Also breaks malformed parent cycles.
		final, ok := finals[k]
		if !ok {
			states[k] = "unknown"
			return "unknown"
		}
		target, ok := index[k]
		if !ok {
			return "partial"
		}
		data := final.Record.Data
		if str(data, "status") != "complete" {
			return "partial"
		}
		revision, count, events, dropped := number(data, "target_revision"), number(data, "record_count"), number(data, "event_count"), number(data, "dropped_events")
		if revision == nil || *revision != target.Record.Revision || count == nil || events == nil || dropped == nil || *dropped != 0 {
			return "partial"
		}
		var required []string
		if json.Unmarshal(data["required_record_ids"], &required) != nil || len(required) == 0 || int64(len(required)) != *count {
			return "partial"
		}
		ids := map[string]bool{}
		for _, id := range required {
			if ids[id] {
				return "partial"
			}
			ids[id] = true
			r, exists := byID[RecordKey{k.installation, id}]
			if !exists || resolve(r, index).unresolved {
				return "partial"
			}
		}
		if !ids[target.Record.RecordID] {
			return "partial"
		}
		var eventCount int64
		seenBatches := map[string]bool{}
		for _, r := range records {
			if r.Device.InstallationID != k.installation || r.Record.Kind == "finalization" {
				continue
			}
			a := resolve(r, index)
			if !belongs(k, a) {
				continue
			}
			// New contextual records invalidate an old finalization's manifest even when
			// the lifecycle revision itself has not changed yet.
			if !ids[r.Record.RecordID] || a.unresolved {
				return "partial"
			}
			if r.Record.Kind == "metric_batch" {
				b, ok := metricBatch(r.Record)
				if !ok {
					return "partial"
				}
				id := canonicalBatchIdentity(r, a, b)
				if !seenBatches[id] {
					seenBatches[id] = true
					eventCount += int64(len(b.Events))
				}
			}
		}
		if eventCount != *events {
			return "partial"
		}
		if k.kind == "conversation" || k.kind == "sweep" {
			for child, r := range index {
				if child == k || child.installation != k.installation || r.Record.Kind != "attempt" && r.Record.Kind != "conversation" {
					continue
				}
				if belongs(k, resolve(r, index)) && check(child) != "complete" {
					return "partial"
				}
			}
		}
		states[k] = "complete"
		return "complete"
	}
	for k := range finals {
		check(k)
	}
	return states
}
func belongs(k entityKey, a attribution) bool {
	switch k.kind {
	case "attempt":
		return a.context.AttemptID == k.id
	case "conversation":
		return a.context.ConversationID == k.id
	case "sweep":
		return a.context.SweepID == k.id
	}
	return false
}
