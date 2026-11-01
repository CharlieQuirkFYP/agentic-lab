// Package storage opens the Go-owned experiment database. Pheme never accesses it.
package storage

import (
	"context"
	"database/sql"
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"
)

func Open(path string) (*sql.DB, error) {
	if path == "" {
		return nil, errors.New("database path is required")
	}
	dsn := path
	if path != ":memory:" {
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			return nil, err
		}
		// Create private files before SQLite opens them (including on permissive umasks).
		file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
		if err != nil {
			return nil, err
		}
		if err := file.Close(); err != nil {
			return nil, err
		}
		absolute, err := filepath.Abs(path)
		if err != nil {
			return nil, err
		}
		uriPath := filepath.ToSlash(absolute)
		if filepath.VolumeName(absolute) != "" {
			uriPath = "/" + uriPath
		}
		dsn = (&url.URL{Scheme: "file", Path: uriPath}).String()
	}
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	// One connection serializes writes and keeps PRAGMA settings consistent.
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, err = db.ExecContext(ctx, `
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS experiments (
 id TEXT PRIMARY KEY,
 status TEXT NOT NULL,
 use_case TEXT NOT NULL,
 dataset TEXT NOT NULL,
 model TEXT NOT NULL,
 created_at TEXT NOT NULL,
 payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS experiments_created ON experiments(created_at DESC, id DESC);
CREATE TABLE IF NOT EXISTS metric_batches (
 experiment_id TEXT NOT NULL REFERENCES experiments(id),
 run_id TEXT NOT NULL,
 sequence_number INTEGER NOT NULL,
 payload TEXT NOT NULL,
 PRIMARY KEY (experiment_id, run_id, sequence_number)
);
CREATE TABLE IF NOT EXISTS voice_metric_batches (
 run_id TEXT NOT NULL,
 sequence_number INTEGER NOT NULL,
 payload TEXT NOT NULL,
 PRIMARY KEY (run_id, sequence_number)
);
`)
	if err != nil {
		_ = db.Close()
		return nil, err
	}
	return db, nil
}
