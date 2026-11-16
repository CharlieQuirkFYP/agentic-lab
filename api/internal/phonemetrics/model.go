package phonemetrics

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"strings"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/metrics"
)

const Source = "device_reported"

var ErrInvalidUpload = errors.New("invalid phone metrics upload")

// Device identifies an app installation, not a person or an Apple device ID.
type Device struct {
	InstallationID string `json:"installation_id"`
	Hardware       string `json:"hardware"`
	OSVersion      string `json:"os_version"`
	AppVersion     string `json:"app_version"`
	Build          string `json:"build"`
}

type Upload struct {
	SchemaVersion int           `json:"schema_version"`
	Device        Device        `json:"device"`
	Batch         metrics.Batch `json:"batch"`
}

type Page struct {
	Source  string          `json:"source"`
	Batches []metrics.Batch `json:"batches"`
	Total   int             `json:"total"`
	Limit   int             `json:"limit"`
	Offset  int             `json:"offset"`
}

func (u Upload) ExperimentID() string {
	hash := sha256.Sum256([]byte(u.Device.InstallationID + "\x00" + u.Batch.RunID))
	return "ios_" + hex.EncodeToString(hash[:16])
}

func (u Upload) Validate() error {
	if u.SchemaVersion != 1 || len(u.Batch.Events) == 0 || len(u.Batch.RunID) > 256 ||
		strings.ContainsRune(u.Batch.RunID, '\x00') || strings.ContainsRune(u.Device.InstallationID, '\x00') {
		return ErrInvalidUpload
	}
	for _, value := range []string{u.Device.InstallationID, u.Device.Hardware, u.Device.OSVersion, u.Device.AppVersion, u.Device.Build} {
		if strings.TrimSpace(value) == "" || len(value) > 256 {
			return ErrInvalidUpload
		}
	}
	if err := u.Batch.ValidateForExperiment(u.ExperimentID()); err != nil {
		return ErrInvalidUpload
	}
	for _, event := range u.Batch.Events {
		if event.ExperimentID != "" && event.ExperimentID != u.ExperimentID() {
			return ErrInvalidUpload
		}
	}
	return nil
}
