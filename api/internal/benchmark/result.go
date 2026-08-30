package benchmark

type Result struct {
	LatencyMS       int     `json:"latency_ms"`
	PeakMemoryMB    int     `json:"peak_memory_mb"`
	TokensPerSecond float64 `json:"tokens_per_second,omitempty"`
	Success         bool    `json:"success"`
	ErrorMessage    string  `json:"error_message,omitempty"`
}
