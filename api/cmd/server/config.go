package main

import (
	"errors"
	"os"
	"strings"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
	"github.com/joho/godotenv"
)

// Load the optional local configuration before reading server settings. Existing
// process variables (including explicitly empty ones) take precedence.
func loadEnvFile(path string) error {
	if err := godotenv.Load(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		// Parser errors can include file contents; keep values out of startup logs.
		return errors.New("could not load .env; check its syntax and file permissions")
	}
	return nil
}

// API_ADDR defaults to loopback because inspection, reset and transcript approval
// are development capabilities, not a multi-user authenticated API. Setting a
// non-loopback address exposes all of them; use access controls and HTTPS there.
// PHEME_VA_URL selects the private Rust host (default http://127.0.0.1:8000).
// API_CORS_ORIGINS is an optional comma-separated list of exact browser origins,
// e.g. http://localhost:5173,http://127.0.0.1:5173. No wildcard or credentials.
type serverConfig struct {
	address      string
	phemeURL     string
	corsOrigins  []string
	databasePath string
}

func loadConfig() serverConfig {
	config := serverConfig{
		address:      "127.0.0.1:8080",
		phemeURL:     pheme.DefaultURL,
		databasePath: "data/experiments.sqlite",
	}
	if address := os.Getenv("API_ADDR"); address != "" {
		config.address = address
	}
	if path := os.Getenv("API_DATABASE_PATH"); path != "" {
		config.databasePath = path
	}
	if phemeURL := os.Getenv("PHEME_VA_URL"); phemeURL != "" {
		config.phemeURL = phemeURL
	}
	if origins := strings.TrimSpace(os.Getenv("API_CORS_ORIGINS")); origins != "" {
		for _, origin := range strings.Split(origins, ",") {
			config.corsOrigins = append(config.corsOrigins, strings.TrimSpace(origin))
		}
	}
	return config
}
