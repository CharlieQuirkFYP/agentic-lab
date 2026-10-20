package handler

import (
	"errors"
	"net/url"
	"strconv"
	"time"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/benchmark"
	"github.com/gin-gonic/gin"
)

func benchmarkQueryValues(c *gin.Context) (url.Values, error) {
	values, err := url.ParseQuery(c.Request.URL.RawQuery)
	if err != nil {
		return nil, benchmark.ErrInvalidQuery
	}
	return values, nil
}

func validateQuery(values url.Values, allowed ...string) error {
	for key, entries := range values {
		found := false
		for _, name := range allowed {
			if key == name {
				found = true
				break
			}
		}
		if !found || len(entries) != 1 || entries[0] == "" {
			return benchmark.ErrInvalidQuery
		}
	}
	return nil
}

func pagination(values url.Values, defaultLimit int) (int, int, error) {
	limit, offset := defaultLimit, 0
	for _, field := range []struct {
		name   string
		target *int
	}{{"limit", &limit}, {"offset", &offset}} {
		if raw, ok := values[field.name]; ok {
			if len(raw) != 1 || raw[0] == "" {
				return 0, 0, benchmark.ErrInvalidQuery
			}
			n, err := strconv.Atoi(raw[0])
			if err != nil {
				return 0, 0, benchmark.ErrInvalidQuery
			}
			*field.target = n
		}
	}
	if limit < 1 || limit > 100 || offset < 0 {
		return 0, 0, benchmark.ErrInvalidQuery
	}
	return limit, offset, nil
}

func experimentQuery(values url.Values) (benchmark.ListQuery, error) {
	if err := validateQuery(values, "limit", "offset", "status", "use_case", "dataset", "model", "created_after", "created_before"); err != nil {
		return benchmark.ListQuery{}, err
	}
	limit, offset, err := pagination(values, 20)
	if err != nil {
		return benchmark.ListQuery{}, err
	}
	q := benchmark.ListQuery{Limit: limit, Offset: offset, Status: benchmark.ExperimentStatus(values.Get("status")), UseCase: benchmark.UseCase(values.Get("use_case")), Dataset: values.Get("dataset"), Model: values.Get("model")}
	for _, field := range []struct {
		name   string
		target **time.Time
	}{{"created_after", &q.CreatedAfter}, {"created_before", &q.CreatedBefore}} {
		if raw := values.Get(field.name); raw != "" {
			parsed, err := time.Parse(time.RFC3339Nano, raw)
			if err != nil {
				return q, errors.Join(benchmark.ErrInvalidQuery, err)
			}
			*field.target = &parsed
		}
	}
	return q, q.Validate()
}
