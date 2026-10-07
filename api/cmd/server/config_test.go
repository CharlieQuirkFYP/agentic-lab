package main

import (
	"reflect"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

func TestConfigDefaultsAreLocal(t *testing.T) {
	t.Setenv("API_ADDR", "")
	t.Setenv("PHEME_VA_URL", "")
	t.Setenv("API_CORS_ORIGINS", "")
	config := loadConfig()
	if config.address != "127.0.0.1:8080" || config.phemeURL != pheme.DefaultURL || len(config.corsOrigins) != 0 {
		t.Fatalf("unexpected defaults: %+v", config)
	}
}

func TestConfigExplicitOverrides(t *testing.T) {
	t.Setenv("API_ADDR", "127.0.0.1:9090")
	t.Setenv("PHEME_VA_URL", "http://127.0.0.1:9000")
	t.Setenv("API_CORS_ORIGINS", " http://localhost:5173, http://127.0.0.1:5173 ")
	config := loadConfig()
	if config.address != "127.0.0.1:9090" || config.phemeURL != "http://127.0.0.1:9000" ||
		!reflect.DeepEqual(config.corsOrigins, []string{"http://localhost:5173", "http://127.0.0.1:5173"}) {
		t.Fatalf("unexpected overrides: %+v", config)
	}
}
