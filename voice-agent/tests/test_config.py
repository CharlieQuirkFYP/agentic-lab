import pytest
from pydantic import ValidationError

from app.config import Settings


def test_environment_settings(monkeypatch):
    monkeypatch.setenv("VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS", "12.5")
    monkeypatch.setenv("VOICE_AGENT_LLAMA_CPP_URL", "http://localhost:9000")
    monkeypatch.setenv("VOICE_AGENT_MODEL_PATH", "/tmp/external-model.gguf")
    settings = Settings.from_env()
    assert settings.analysis_timeout_seconds == 12.5
    assert str(settings.llama_cpp_url) == "http://localhost:9000/"
    assert str(settings.model_path) == "/tmp/external-model.gguf"


@pytest.mark.parametrize("value", ["0", "-1", "nan", "inf", "invalid"])
def test_invalid_timeout_fails_at_startup(monkeypatch, value):
    monkeypatch.setenv("VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS", value)
    with pytest.raises(ValidationError):
        Settings.from_env()


def test_invalid_runtime_url_fails_at_startup(monkeypatch):
    monkeypatch.setenv("VOICE_AGENT_LLAMA_CPP_URL", "not a URL")
    with pytest.raises(ValidationError):
        Settings.from_env()


def test_analyzer_settings(monkeypatch):
    monkeypatch.setenv("VOICE_AGENT_ANALYZER", "llama_cpp")
    monkeypatch.setenv("VOICE_AGENT_MAX_OUTPUT_TOKENS", "256")
    settings = Settings.from_env()
    assert settings.analyzer_backend == "llama_cpp"
    assert settings.max_output_tokens == 256


@pytest.mark.parametrize(
    "env,value",
    [
        ("VOICE_AGENT_ANALYZER", "unknown"),
        ("VOICE_AGENT_MAX_OUTPUT_TOKENS", "0"),
        ("VOICE_AGENT_MAX_OUTPUT_TOKENS", "4097"),
    ],
)
def test_invalid_analyzer_settings(monkeypatch, env, value):
    monkeypatch.setenv(env, value)
    with pytest.raises(ValidationError):
        Settings.from_env()
