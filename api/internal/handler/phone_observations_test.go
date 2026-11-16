package handler

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phonemetrics"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/phoneobservations"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/storage"
	"github.com/gin-gonic/gin"
)

const observationFixture = `{"schema_version":1,"device":{"installation_id":"fixture","hardware":"test-fixture-not-a-phone","os_version":"test","app_version":"test","build":"test"},"records":[{"schema_version":1,"record_id":"record-1","producer_stream_id":"stream-1","record_sequence":0,"timestamp_ms":1000,"kind":"conversation","entity_id":"conversation-1","revision":0,"context":{},"data":{"status":"active","source":"manual","stage":"reasoning"}}]}`

func observationRouter(t *testing.T) (*gin.Engine, *int) {
	t.Helper()
	db, err := storage.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	gin.SetMode(gin.TestMode)
	router := gin.New()
	cors, err := NewBenchmarkCORS([]string{"http://localhost:5173"})
	if err != nil {
		t.Fatal(err)
	}
	notifications := 0
	group := router.Group("/api/v1/phone", cors)
	NewPhoneObservationsHandler(phoneobservations.NewService(phoneobservations.NewStore(db)), func() { notifications++ }).RegisterRoutes(group)
	NewPhoneMetricsHandler(phonemetrics.NewStore(db), nil).RegisterRoutes(group)
	return router, &notifications
}

func observationRequest(router http.Handler, method, path, body string) *httptest.ResponseRecorder {
	request := httptest.NewRequest(method, "/api/v1/phone"+path, strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	return response
}

func observationJSON(t *testing.T, response *httptest.ResponseRecorder, status int, target any) {
	t.Helper()
	if response.Code != status {
		t.Fatalf("HTTP status = %d, want %d: %s", response.Code, status, response.Body.String())
	}
	if err := json.Unmarshal(response.Body.Bytes(), target); err != nil {
		t.Fatalf("decode response: %v: %s", err, response.Body.String())
	}
}

func observationBody(t *testing.T, change func(map[string]any, map[string]any)) string {
	t.Helper()
	var upload map[string]any
	if err := json.Unmarshal([]byte(observationFixture), &upload); err != nil {
		t.Fatal(err)
	}
	record := upload["records"].([]any)[0].(map[string]any)
	change(upload, record)
	body, err := json.Marshal(upload)
	if err != nil {
		t.Fatal(err)
	}
	return string(body)
}

func TestPhoneObservationsHTTPRetryAndAtomicConflict(t *testing.T) {
	router, notifications := observationRouter(t)
	var firstReceipt int64
	for i := range 2 {
		var accepted phoneobservations.Accepted
		observationJSON(t, observationRequest(router, http.MethodPost, "/observations", observationFixture), http.StatusAccepted, &accepted)
		if accepted.Status != "accepted" || accepted.RecordCount != 1 || !reflect.DeepEqual(accepted.AcceptedRecordIDs, []string{"record-1"}) {
			t.Fatalf("acknowledgement = %+v", accepted)
		}
		var page phoneobservations.Page
		observationJSON(t, observationRequest(router, http.MethodGet, "/observations", ""), http.StatusOK, &page)
		if page.Total != 1 || len(page.Records) != 1 || page.Records[0].Device.InstallationID != "fixture" || page.Records[0].ReceivedAtMS <= 0 {
			t.Fatalf("stored retry = %+v", page)
		}
		if i == 0 {
			firstReceipt = page.Records[0].ReceivedAtMS
		} else if page.Records[0].ReceivedAtMS != firstReceipt {
			t.Fatal("retry changed original receipt time")
		}
	}
	for _, identity := range []string{"record", "stream_sequence", "entity_revision", "device"} {
		t.Run(identity, func(t *testing.T) {
			body := observationBody(t, func(upload, record map[string]any) {
				switch identity {
				case "record":
					record["record_sequence"], record["entity_id"] = 8, "other-conversation"
				case "stream_sequence":
					record["record_id"], record["entity_id"] = "other-record", "other-conversation"
				case "entity_revision":
					record["record_id"], record["record_sequence"] = "other-record", 8
				case "device":
					upload["device"].(map[string]any)["build"] = "changed"
				}
				// Insert a valid new record before the conflict to prove the entire chunk rolls back.
				fresh := map[string]any{}
				for k, v := range record {
					fresh[k] = v
				}
				fresh["record_id"], fresh["record_sequence"], fresh["entity_id"] = "must-roll-back", 9, "must-roll-back"
				upload["records"] = []any{fresh, record}
			})
			var failure map[string]string
			observationJSON(t, observationRequest(router, http.MethodPost, "/observations", body), http.StatusConflict, &failure)
			if failure["error"] != "phone observation identity already has different content" {
				t.Fatalf("conflict error = %v", failure)
			}
			var page phoneobservations.Page
			observationJSON(t, observationRequest(router, http.MethodGet, "/observations", ""), http.StatusOK, &page)
			if page.Total != 1 || page.Records[0].Record.RecordID != "record-1" || *notifications != 2 {
				t.Fatalf("conflict leaked writes or notifications: page=%+v notifications=%d", page, *notifications)
			}
		})
	}
	otherInstallation := observationBody(t, func(upload, _ map[string]any) {
		upload["device"].(map[string]any)["installation_id"] = "other-installation"
	})
	var accepted phoneobservations.Accepted
	observationJSON(t, observationRequest(router, http.MethodPost, "/observations", otherInstallation), http.StatusAccepted, &accepted)
	if *notifications != 3 {
		t.Fatalf("accepted uploads notified %d times, want 3", *notifications)
	}
}

func TestPhoneObservationsHTTPStrictUpload(t *testing.T) {
	router, notifications := observationRouter(t)
	invalid := map[string]string{
		"empty": "", "null": "null", "array": "[]", "object": "{}",
		"second_object": observationFixture + "{}", "trailing_null": observationFixture + "null", "trailing_garbage": observationFixture + "!",
	}
	for scope, fields := range map[string][]string{
		"upload": {"schema_version", "device", "records"},
		"device": {"installation_id", "hardware", "os_version", "app_version", "build"},
		"record": {"schema_version", "record_id", "producer_stream_id", "record_sequence", "timestamp_ms", "kind", "entity_id", "revision", "context", "data"},
	} {
		for _, field := range append(fields, "unknown") {
			for _, mode := range []string{"missing", "null"} {
				invalid[scope+"/"+field+"/"+mode] = observationBody(t, func(upload, record map[string]any) {
					target := upload
					if scope == "record" {
						target = record
					} else if scope == "device" {
						target = upload["device"].(map[string]any)
					}
					if field == "unknown" {
						target["unknown"] = true
					} else if mode == "missing" {
						delete(target, field)
					} else {
						target[field] = nil
					}
				})
			}
		}
	}
	for name, change := range map[string]func(map[string]any, map[string]any){
		"unknown_context":     func(_, r map[string]any) { r["context"] = map[string]any{"unknown": "value"} },
		"wrong_schema":        func(u, _ map[string]any) { u["schema_version"] = 2 },
		"wrong_record_schema": func(_, r map[string]any) { r["schema_version"] = 2 },
		"empty_records":       func(u, _ map[string]any) { u["records"] = []any{} },
		"null_record":         func(u, _ map[string]any) { u["records"] = []any{nil} },
		"negative_sequence":   func(_, r map[string]any) { r["record_sequence"] = -1 },
		"fractional_sequence": func(_, r map[string]any) { r["record_sequence"] = 0.5 },
		"negative_timestamp":  func(_, r map[string]any) { r["timestamp_ms"] = -1 },
		"negative_revision":   func(_, r map[string]any) { r["revision"] = -1 },
		"unknown_kind":        func(_, r map[string]any) { r["kind"] = "inference" },
		"invalid_status":      func(_, r map[string]any) { r["data"] = map[string]any{"status": "done"} },
		"content":             func(_, r map[string]any) { r["data"].(map[string]any)["transcript"] = "must not be stored" },
		"too_many_records": func(u, r map[string]any) {
			records := make([]any, phoneobservations.MaxRecords+1)
			for i := range records {
				records[i] = r
			}
			u["records"] = records
		},
	} {
		invalid[name] = observationBody(t, change)
	}
	for name, body := range invalid {
		t.Run(name, func(t *testing.T) {
			var failure map[string]string
			observationJSON(t, observationRequest(router, http.MethodPost, "/observations", body), http.StatusBadRequest, &failure)
			if failure["error"] != "invalid phone observation upload" {
				t.Fatalf("validation error = %v", failure)
			}
		})
	}
	for name, body := range map[string]string{
		"oversize_body":                strings.Repeat(" ", (4<<20)+1),
		"oversize_trailing_whitespace": observationFixture + strings.Repeat(" ", 4<<20),
	} {
		t.Run(name, func(t *testing.T) {
			var failure map[string]string
			observationJSON(t, observationRequest(router, http.MethodPost, "/observations", body), http.StatusRequestEntityTooLarge, &failure)
		})
	}
	var page phoneobservations.Page
	observationJSON(t, observationRequest(router, http.MethodGet, "/observations", ""), http.StatusOK, &page)
	if page.Total != 0 || *notifications != 0 {
		t.Fatalf("invalid upload had side effects: total=%d notifications=%d", page.Total, *notifications)
	}
}

func TestPhoneObservationsHTTPInvalidQueries(t *testing.T) {
	router, _ := observationRouter(t)
	for _, endpoint := range []string{"/observations", "/legacy-observations", "/overview"} {
		queries := []string{
			"unknown=1", "installation_id=", "installation_id=one&installation_id=two", "installation_id=%00",
			"since_ms=-1", "until_ms=-1", "since_ms=1.5", "since_ms=9223372036854775808", "since_ms=2&until_ms=1",
			"source=mock", "stage=unknown", "outcome=complete", "as_of=not-a-snapshot", "installation_id=%zz", "installation_id=one;two",
		}
		if endpoint == "/overview" {
			queries = append(queries, "limit=1", "cursor=anything", "run_id=legacy")
		} else {
			queries = append(queries, "limit=0", "limit=-1", "limit=101", "limit=1.5", "limit=one", "limit=1&limit=2", "cursor=invalid")
		}
		if endpoint == "/observations" {
			queries = append(queries, "run_id=legacy")
		}
		for _, query := range queries {
			t.Run(endpoint+"/"+query, func(t *testing.T) {
				var failure map[string]string
				observationJSON(t, observationRequest(router, http.MethodGet, endpoint+"?"+query, ""), http.StatusBadRequest, &failure)
				if failure["error"] != "invalid phone observation query" {
					t.Fatalf("query error = %v", failure)
				}
			})
		}
	}
}

func TestPhoneObservationsHTTPSnapshotPaginationAndFilters(t *testing.T) {
	router, _ := observationRouter(t)
	for i := 1; i <= 3; i++ {
		body := observationBody(t, func(_, r map[string]any) {
			r["record_id"], r["entity_id"], r["record_sequence"], r["timestamp_ms"] = fmt.Sprintf("record-%d", i), fmt.Sprintf("conversation-%d", i), i, i*1000
			if i == 1 {
				r["context"] = map[string]any{"configuration_id": "configuration-1", "sweep_id": "sweep-1"}
			} else {
				r["data"] = map[string]any{"status": "succeeded", "source": "acceptance", "stage": "transcription"}
			}
		})
		var accepted phoneobservations.Accepted
		observationJSON(t, observationRequest(router, http.MethodPost, "/observations", body), http.StatusAccepted, &accepted)
	}
	var first phoneobservations.Page
	observationJSON(t, observationRequest(router, http.MethodGet, "/observations?limit=1&installation_id=fixture", ""), http.StatusOK, &first)
	if first.SchemaVersion != 1 || first.AsOf == "" || first.Total != 3 || len(first.Records) != 1 || first.NextCursor == nil {
		t.Fatalf("first page = %+v", first)
	}
	late := observationBody(t, func(_, r map[string]any) {
		r["record_id"], r["entity_id"], r["record_sequence"], r["timestamp_ms"] = "record-4", "conversation-4", 4, 4000
	})
	var accepted phoneobservations.Accepted
	observationJSON(t, observationRequest(router, http.MethodPost, "/observations", late), http.StatusAccepted, &accepted)
	seen := []string{first.Records[0].Record.RecordID}
	for cursor := first.NextCursor; cursor != nil; {
		if len(seen) > 3 {
			t.Fatal("snapshot pagination did not terminate")
		}
		var page phoneobservations.Page
		observationJSON(t, observationRequest(router, http.MethodGet, "/observations?limit=1&installation_id=fixture&cursor="+url.QueryEscape(*cursor), ""), http.StatusOK, &page)
		if page.AsOf != first.AsOf || page.Total != 3 || len(page.Records) != 1 {
			t.Fatalf("snapshot changed during pagination: %+v", page)
		}
		seen = append(seen, page.Records[0].Record.RecordID)
		cursor = page.NextCursor
	}
	if !reflect.DeepEqual(seen, []string{"record-1", "record-2", "record-3"}) {
		t.Fatalf("snapshot record IDs = %v", seen)
	}
	var fresh phoneobservations.Page
	observationJSON(t, observationRequest(router, http.MethodGet, "/observations", ""), http.StatusOK, &fresh)
	if fresh.Total != 4 || fresh.NextCursor != nil {
		t.Fatalf("fresh snapshot = %+v", fresh)
	}
	for _, query := range []string{
		"limit=1&installation_id=other&cursor=" + url.QueryEscape(*first.NextCursor),
		"limit=1&installation_id=fixture&as_of=" + url.QueryEscape(fresh.AsOf) + "&cursor=" + url.QueryEscape(*first.NextCursor),
	} {
		var failure map[string]string
		observationJSON(t, observationRequest(router, http.MethodGet, "/observations?"+query, ""), http.StatusBadRequest, &failure)
	}
	var failure map[string]string
	observationJSON(t, observationRequest(router, http.MethodGet, "/legacy-observations?installation_id=fixture&cursor="+url.QueryEscape(*first.NextCursor), ""), http.StatusBadRequest, &failure)
	for _, filter := range []string{"conversation_id=conversation-1", "configuration_id=configuration-1", "sweep_id=sweep-1", "source=manual", "stage=reasoning", "outcome=active", "since_ms=1000&until_ms=1000"} {
		t.Run(filter, func(t *testing.T) {
			query := "?as_of=" + url.QueryEscape(first.AsOf) + "&" + filter
			var page phoneobservations.Page
			observationJSON(t, observationRequest(router, http.MethodGet, "/observations"+query, ""), http.StatusOK, &page)
			if page.Total != 1 || len(page.Records) != 1 || page.Records[0].Record.EntityID != "conversation-1" {
				t.Fatalf("filtered page = %+v", page)
			}
			var overview phoneobservations.Overview
			observationJSON(t, observationRequest(router, http.MethodGet, "/overview"+query, ""), http.StatusOK, &overview)
			if overview.AsOf != first.AsOf || overview.Coverage.RecordCount != 1 || overview.Counts.Conversations.Total != 1 || overview.CurrentConversation == nil || overview.CurrentConversation.EntityID != "conversation-1" {
				t.Fatalf("filtered overview = %+v", overview)
			}
		})
	}
	var overview phoneobservations.Overview
	observationJSON(t, observationRequest(router, http.MethodGet, "/overview?as_of="+url.QueryEscape(first.AsOf), ""), http.StatusOK, &overview)
	if overview.SchemaVersion != 1 || overview.Source != "device_reported" || overview.Coverage.RecordCount != 3 || overview.Counts.Conversations.Total != 3 || overview.Coverage.LegacyConversationCount != nil || !overview.Coverage.RetainedHistory {
		t.Fatalf("snapshot overview = %+v", overview)
	}
}

func TestPhoneObservationsHTTPLegacySnapshot(t *testing.T) {
	router, _ := observationRouter(t)
	for i := 1; i <= 2; i++ {
		body := strings.ReplaceAll(phoneFixture, "fixture-run", fmt.Sprintf("legacy-%d", i))
		var accepted map[string]any
		observationJSON(t, observationRequest(router, http.MethodPost, "/metrics", body), http.StatusAccepted, &accepted)
	}
	var first phoneobservations.LegacyPage
	observationJSON(t, observationRequest(router, http.MethodGet, "/legacy-observations?limit=1", ""), http.StatusOK, &first)
	if first.SchemaVersion != 1 || first.Total != 2 || len(first.Batches) != 1 || first.NextCursor == nil || first.Batches[0].Device == nil || first.Batches[0].Device.InstallationID != "fixture" || first.Batches[0].ReceivedAtMS <= 0 {
		t.Fatalf("legacy first page = %+v", first)
	}
	var accepted map[string]any
	observationJSON(t, observationRequest(router, http.MethodPost, "/metrics", strings.ReplaceAll(phoneFixture, "fixture-run", "legacy-3")), http.StatusAccepted, &accepted)
	var second phoneobservations.LegacyPage
	observationJSON(t, observationRequest(router, http.MethodGet, "/legacy-observations?limit=1&cursor="+url.QueryEscape(*first.NextCursor), ""), http.StatusOK, &second)
	if second.AsOf != first.AsOf || second.Total != 2 || len(second.Batches) != 1 || second.NextCursor != nil || second.Batches[0].Batch.RunID != "legacy-2" {
		t.Fatalf("legacy continuation = %+v", second)
	}
	var overview phoneobservations.Overview
	observationJSON(t, observationRequest(router, http.MethodGet, "/overview?as_of="+url.QueryEscape(first.AsOf), ""), http.StatusOK, &overview)
	if overview.Coverage.LegacyBatchCount != 2 || overview.Coverage.MetricBatchCount != 2 || overview.Coverage.LegacyConversationCount != nil || overview.Counts.Conversations.Total != 0 || overview.CurrentConversation != nil {
		t.Fatalf("legacy snapshot overview = %+v", overview)
	}
	for _, filter := range []string{"conversation_id=unknown", "configuration_id=unknown", "sweep_id=unknown", "source=manual", "outcome=succeeded", "stage=reasoning"} {
		var page phoneobservations.LegacyPage
		observationJSON(t, observationRequest(router, http.MethodGet, "/legacy-observations?"+filter, ""), http.StatusOK, &page)
		if page.Total != 0 || page.Batches == nil || page.NextCursor != nil {
			t.Fatalf("legacy unknown attribution for %q = %+v", filter, page)
		}
	}
	var page phoneobservations.LegacyPage
	observationJSON(t, observationRequest(router, http.MethodGet, "/legacy-observations?installation_id=fixture&run_id=legacy-1&since_ms=1000&until_ms=1000", ""), http.StatusOK, &page)
	if page.Total != 1 || len(page.Batches) != 1 || page.Batches[0].Batch.RunID != "legacy-1" {
		t.Fatalf("legacy filtered page = %+v", page)
	}
}

func TestPhoneObservationsHTTPCORS(t *testing.T) {
	router, notifications := observationRouter(t)
	for endpoint, method := range map[string]string{"/observations": http.MethodPost, "/legacy-observations": http.MethodGet, "/overview": http.MethodGet} {
		request := httptest.NewRequest(http.MethodOptions, "/api/v1/phone"+endpoint, nil)
		request.Header.Set("Origin", "http://localhost:5173")
		request.Header.Set("Access-Control-Request-Method", method)
		request.Header.Set("Access-Control-Request-Headers", "Content-Type")
		response := httptest.NewRecorder()
		router.ServeHTTP(response, request)
		if response.Code != http.StatusNoContent || response.Header().Get("Access-Control-Allow-Origin") != "http://localhost:5173" {
			t.Fatalf("preflight %s = %d: %v", endpoint, response.Code, response.Header())
		}
	}
	request := httptest.NewRequest(http.MethodPost, "/api/v1/phone/observations", strings.NewReader(observationFixture))
	request.Header.Set("Origin", "http://untrusted.example")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden || response.Header().Get("Access-Control-Allow-Origin") != "" || *notifications != 0 {
		t.Fatalf("untrusted browser upload = %d, notifications=%d", response.Code, *notifications)
	}
	var page phoneobservations.Page
	observationJSON(t, observationRequest(router, http.MethodGet, "/observations", ""), http.StatusOK, &page)
	if page.Total != 0 {
		t.Fatal("untrusted upload was persisted")
	}
}
