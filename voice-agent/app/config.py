"""Validated environment configuration; importing this module performs no I/O."""

import os
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, HttpUrl


class Settings(BaseModel):
    model_config = ConfigDict(extra="forbid", allow_inf_nan=False)

    analyzer_backend: Literal["unavailable", "llama_cpp"] = "unavailable"
    max_output_tokens: int = Field(default=512, ge=64, le=4096)
    analysis_timeout_seconds: float = Field(default=55, gt=0)
    llama_cpp_url: HttpUrl = HttpUrl("http://127.0.0.1:8081")
    model_path: Path | None = None

    @classmethod
    def from_env(cls) -> "Settings":
        values = {
            field: os.environ[env]
            for field, env in {
                "analyzer_backend": "VOICE_AGENT_ANALYZER",
                "max_output_tokens": "VOICE_AGENT_MAX_OUTPUT_TOKENS",
                "analysis_timeout_seconds": "VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS",
                "llama_cpp_url": "VOICE_AGENT_LLAMA_CPP_URL",
                "model_path": "VOICE_AGENT_MODEL_PATH",
            }.items()
            if env in os.environ
        }
        return cls.model_validate(values)
