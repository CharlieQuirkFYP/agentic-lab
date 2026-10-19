package storage

import (
	"path/filepath"
	"testing"
)

func TestDatabasePathIsLiteralAndPersists(t *testing.T) {
	// URI/query characters must remain filename characters, not driver options.
	path := filepath.Join(t.TempDir(), "folder with spaces", "experiments ? #.sqlite")
	db, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(t.Context(), `INSERT INTO experiments VALUES ('saved','completed','incident-reporting','','','2026-10-19','{}')`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	db, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var count int
	if err := db.QueryRowContext(t.Context(), `SELECT count(*) FROM experiments WHERE id='saved'`).Scan(&count); err != nil || count != 1 {
		t.Fatalf("count=%d error=%v", count, err)
	}
}
