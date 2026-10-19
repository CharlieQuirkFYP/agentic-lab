package metrics

import (
	"context"
	"errors"
	"sort"
)

var ErrInvalidQuery = errors.New("invalid metrics query")

// A zero limit preserves the legacy unpaginated response. New clients should
// always supply a bounded limit; offsets require an explicit limit.
type Query struct {
	Limit  int
	Offset int
	RunID  string
	Name   string
}

type Page struct {
	ExperimentID string  `json:"experiment_id"`
	Batches      []Batch `json:"batches"`
	Total        int     `json:"total"`
	Limit        int     `json:"limit"`
	Offset       int     `json:"offset"`
}

func (q Query) Validate() error {
	if q.Limit < 0 || q.Limit > 100 || q.Offset < 0 || q.Limit == 0 && q.Offset != 0 {
		return ErrInvalidQuery
	}
	return nil
}

func (s *Service) List(ctx context.Context, id string, q Query) (Page, error) {
	if err := q.Validate(); err != nil {
		return Page{}, err
	}
	batches, err := s.GetByExperimentID(ctx, id)
	if err != nil {
		return Page{}, err
	}
	selected := []Batch{}
	for _, b := range batches {
		if q.RunID != "" && b.RunID != q.RunID {
			continue
		}
		if q.Name != "" {
			events := []Event{}
			for _, e := range b.Events {
				if e.Name == q.Name {
					events = append(events, e)
				}
			}
			if len(events) == 0 {
				continue
			}
			b.Events = events
		}
		selected = append(selected, b)
	}
	sort.Slice(selected, func(i, j int) bool {
		if selected[i].RunID == selected[j].RunID {
			return selected[i].SequenceNumber < selected[j].SequenceNumber
		}
		return selected[i].RunID < selected[j].RunID
	})
	page := Page{ExperimentID: id, Batches: []Batch{}, Total: len(selected), Limit: q.Limit, Offset: q.Offset}
	if q.Offset < len(selected) {
		end := len(selected)
		if q.Limit > 0 {
			end = min(end, q.Offset+min(q.Limit, len(selected)-q.Offset))
		}
		page.Batches = selected[q.Offset:end]
	}
	return page, nil
}
