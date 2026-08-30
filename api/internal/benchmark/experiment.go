package benchmark

import "time"

type UseCase string

const (
	UseCaseIncidentReporting      UseCase = "incident-reporting"
	UseCaseInterviewAssistant     UseCase = "interview-assistant"
	UseCaseLicensePlateMonitoring UseCase = "license-plate-monitoring"
)

func (u UseCase) IsSupported() bool {
	switch u {
	case UseCaseIncidentReporting, UseCaseInterviewAssistant, UseCaseLicensePlateMonitoring:
		return true
	default:
		return false
	}
}

type ExperimentStatus string

const (
	ExperimentStatusQueued    ExperimentStatus = "queued"
	ExperimentStatusRunning   ExperimentStatus = "running"
	ExperimentStatusCompleted ExperimentStatus = "completed"
	ExperimentStatusFailed    ExperimentStatus = "failed"
)

type ModelConfig struct {
	Name         string `json:"name,omitempty"`
	Version      string `json:"version,omitempty"`
	Quantization string `json:"quantization,omitempty"`
	Runtime      string `json:"runtime,omitempty"`
}

type EnvironmentProfile struct {
	Name     string `json:"name,omitempty"`
	Type     string `json:"type,omitempty"`
	CPUCores int    `json:"cpu_cores,omitempty"`
	MemoryMB int    `json:"memory_mb,omitempty"`
	GPUInfo  string `json:"gpu_info,omitempty"`
}

type Experiment struct {
	ID                 string             `json:"id"`
	UseCase            UseCase            `json:"use_case"`
	Dataset            string             `json:"dataset,omitempty"`
	ModelConfig        ModelConfig        `json:"model_config"`
	EnvironmentProfile EnvironmentProfile `json:"environment_profile"`
	Status             ExperimentStatus   `json:"status"`
	CreatedAt          time.Time          `json:"created_at"`
	StartedAt          *time.Time         `json:"started_at,omitempty"`
	CompletedAt        *time.Time         `json:"completed_at,omitempty"`
	Result             *Result            `json:"result,omitempty"`
}

type CreateExperimentRequest struct {
	UseCase            UseCase            `json:"use_case"`
	Dataset            string             `json:"dataset,omitempty"`
	ModelConfig        ModelConfig        `json:"model_config"`
	EnvironmentProfile EnvironmentProfile `json:"environment_profile"`
}

type CreateExperimentResponse struct {
	ID     string           `json:"id"`
	Status ExperimentStatus `json:"status"`
}
