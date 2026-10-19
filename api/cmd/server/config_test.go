package main

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

func unsetConfigEnv(t *testing.T) {
	t.Helper()
	for _, key := range []string{"API_ADDR", "API_CORS_ORIGINS", "API_DATABASE_PATH", "PHEME_VA_URL"} {
		// Register restoration of the original value before unsetting it.
		t.Setenv(key, "")
		if err := os.Unsetenv(key); err != nil {
			t.Fatal(err)
		}
	}
}

func TestEnvExampleLoadsServerSettings(t *testing.T) {
	unsetConfigEnv(t)
	if err := loadEnvFile(filepath.Join("..", "..", ".env.example")); err != nil {
		t.Fatal(err)
	}
	config := loadConfig()
	if config.address != "127.0.0.1:8080" || config.phemeURL != pheme.DefaultURL || config.databasePath != "data/experiments.sqlite" ||
		!reflect.DeepEqual(config.corsOrigins, []string{"http://localhost:5173", "http://127.0.0.1:5173"}) {
		t.Fatalf("unexpected example config: %+v", config)
	}
}

func TestEnvFilePreservesProcessOverrides(t *testing.T) {
	unsetConfigEnv(t)
	t.Setenv("API_ADDR", "127.0.0.1:9090")
	t.Setenv("API_CORS_ORIGINS", "") // Explicitly disable browser origins.
	path := filepath.Join(t.TempDir(), ".env")
	if err := os.WriteFile(path, []byte("API_ADDR=127.0.0.1:8081\nAPI_CORS_ORIGINS=http://localhost:5173\nAPI_DATABASE_PATH=\"data/test experiments.sqlite\"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := loadEnvFile(path); err != nil {
		t.Fatal(err)
	}
	config := loadConfig()
	if config.address != "127.0.0.1:9090" || len(config.corsOrigins) != 0 || config.databasePath != "data/test experiments.sqlite" {
		t.Fatalf("unexpected overrides: %+v", config)
	}
}

func TestEnvFileMissingIsOptionalAndMalformedFails(t *testing.T) {
	unsetConfigEnv(t)
	path := filepath.Join(t.TempDir(), ".env")
	if err := loadEnvFile(path); err != nil {
		t.Fatalf("missing file should be optional: %v", err)
	}
	if err := os.WriteFile(path, []byte("API_ADDR=127.0.0.1:9091\nPHEME_VA_URL=\"unterminated\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := loadEnvFile(path); err == nil {
		t.Fatal("malformed .env should fail startup")
	}
	if _, exists := os.LookupEnv("API_ADDR"); exists {
		t.Fatal("malformed file partially changed the environment")
	}
}

func TestConfigDefaultsAreLocal(t *testing.T) {
	t.Setenv("API_ADDR", "")
	t.Setenv("PHEME_VA_URL", "")
	t.Setenv("API_CORS_ORIGINS", "")
	t.Setenv("API_DATABASE_PATH", "")
	config := loadConfig()
	if config.address != "127.0.0.1:8080" || config.phemeURL != pheme.DefaultURL || len(config.corsOrigins) != 0 || config.databasePath != "data/experiments.sqlite" {
		t.Fatalf("unexpected defaults: %+v", config)
	}
}

func TestConfigExplicitOverrides(t *testing.T) {
	t.Setenv("API_ADDR", "127.0.0.1:9090")
	t.Setenv("PHEME_VA_URL", "http://127.0.0.1:9000")
	t.Setenv("API_CORS_ORIGINS", " http://localhost:5173, http://127.0.0.1:5173 ")
	t.Setenv("API_DATABASE_PATH", "/tmp/benchmark-tests.sqlite")
	config := loadConfig()
	if config.address != "127.0.0.1:9090" || config.phemeURL != "http://127.0.0.1:9000" ||
		!reflect.DeepEqual(config.corsOrigins, []string{"http://localhost:5173", "http://127.0.0.1:5173"}) || config.databasePath != "/tmp/benchmark-tests.sqlite" {
		t.Fatalf("unexpected overrides: %+v", config)
	}
}
