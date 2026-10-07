package service

import (
	"context"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/pheme"
)

// VoiceClient is the integration boundary; Pheme owns all conversation rules.
type VoiceClient interface {
	StartTurn(context.Context, pheme.Input) (*pheme.Response, error)
	Submit(context.Context, string, pheme.Input) (*pheme.Response, error)
	TurnStatus(context.Context, string) (*pheme.Response, error)
	Cancel(context.Context, string) (*pheme.Response, error)
	Reset(context.Context) (*pheme.Response, error)
	Inspect(context.Context) (*pheme.Response, error)
	TestReply(context.Context, pheme.Input) (*pheme.Response, error)
	Transcribe(context.Context, pheme.Input) (*pheme.Response, error)
}

type VoiceService struct {
	client VoiceClient
}

func NewVoiceService(client VoiceClient) *VoiceService {
	return &VoiceService{client: client}
}

func (s *VoiceService) StartTurn(ctx context.Context, input pheme.Input) (*pheme.Response, error) {
	return s.client.StartTurn(ctx, input)
}

func (s *VoiceService) Submit(ctx context.Context, turnID string, input pheme.Input) (*pheme.Response, error) {
	return s.client.Submit(ctx, turnID, input)
}

func (s *VoiceService) TurnStatus(ctx context.Context, turnID string) (*pheme.Response, error) {
	return s.client.TurnStatus(ctx, turnID)
}

func (s *VoiceService) Cancel(ctx context.Context, turnID string) (*pheme.Response, error) {
	return s.client.Cancel(ctx, turnID)
}

func (s *VoiceService) Reset(ctx context.Context) (*pheme.Response, error) {
	return s.client.Reset(ctx)
}

func (s *VoiceService) Inspect(ctx context.Context) (*pheme.Response, error) {
	return s.client.Inspect(ctx)
}

func (s *VoiceService) TestReply(ctx context.Context, input pheme.Input) (*pheme.Response, error) {
	return s.client.TestReply(ctx, input)
}

func (s *VoiceService) Transcribe(ctx context.Context, input pheme.Input) (*pheme.Response, error) {
	return s.client.Transcribe(ctx, input)
}
