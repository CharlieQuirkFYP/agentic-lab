package main

import (
	"os"
	"strings"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

// API_ADDR defaults to loopback because inspection, reset and transcript approval
// are development capabilities, not a multi-user authenticated API. Setting a
// non-loopback address exposes all of them; use access controls and HTTPS there.
// PHEME_VA_URL selects the private Rust host (default http://127.0.0.1:8000).
// API_CORS_ORIGINS is an optional comma-separated list of exact browser origins,
// e.g. http://localhost:5173,http://127.0.0.1:5173. No wildcard or credentials.
type serverConfig struct {
	address     string
	phemeURL    string
	corsOrigins []string
}

func loadConfig() serverConfig {
	config := serverConfig{
		address:  "127.0.0.1:8080",
		phemeURL: pheme.DefaultURL,
	}
	if address := os.Getenv("API_ADDR"); address != "" {
		config.address = address
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
