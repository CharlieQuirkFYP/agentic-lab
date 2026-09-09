"""Validated environment configuration; importing this module performs no I/O."""

import os
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field, HttpUrl


class Settings(BaseModel):
    model_config = ConfigDict(extra="forbid", allow_inf_nan=False)

    analysis_timeout_seconds: float = Field(default=55, gt=0)
    llama_cpp_url: HttpUrl = HttpUrl("http://127.0.0.1:8081")
    model_path: Path | None = None

    @classmethod
    def from_env(cls) -> "Settings":
        values = {
            field: os.environ[env]
            for field, env in {
                "analysis_timeout_seconds": "VOICE_AGENT_ANALYSIS_TIMEOUT_SECONDS",
                "llama_cpp_url": "VOICE_AGENT_LLAMA_CPP_URL",
                "model_path": "VOICE_AGENT_MODEL_PATH",
            }.items()
            if env in os.environ
        }
        return cls.model_validate(values)
