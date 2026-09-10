"""Adapter for the pinned llama.cpp single-model local HTTP server."""

import asyncio
import json
import logging
from pathlib import Path
from time import perf_counter

import httpx
from pydantic import ValidationError

from app.config import Settings
from app.incident_reporting.models import AnalyzeRequest, IncidentReport
from app.incident_reporting.prompts import EXAMPLES, PROMPT_VERSION, SYSTEM_PROMPT
from app.incident_reporting.service import InvalidModelResponse, RuntimeUnavailable

logger = logging.getLogger("uvicorn.error")


class LlamaCppAnalyzer:
    def __init__(self, settings: Settings, client: httpx.AsyncClient | None = None):
        self.settings = settings
        self._owns_client = client is None
        self.client = (
            client
            if client is not None
            else httpx.AsyncClient(
                base_url=str(settings.llama_cpp_url).rstrip("/") + "/",
                timeout=settings.analysis_timeout_seconds,
                trust_env=False,
            )
        )

    async def aclose(self) -> None:
        if self._owns_client:
            await self.client.aclose()

    async def _json(self, method: str, path: str, **kwargs) -> dict:
        try:
            response = await self.client.request(method, path, **kwargs)
        except httpx.TimeoutException as exc:
            raise TimeoutError from exc
        except httpx.RequestError as exc:
            raise RuntimeUnavailable from exc
        if response.status_code in (429, 502, 503):
            raise RuntimeUnavailable
        if response.status_code == 504:
            raise TimeoutError
        if response.status_code != 200:
            # Includes incompatible runtime configuration; never expose the body.
            raise RuntimeError("Runtime request failed")
        try:
            value = response.json()
        except ValueError as exc:
            raise InvalidModelResponse from exc
        if not isinstance(value, dict):
            raise InvalidModelResponse
        return value

    async def is_ready(self) -> bool:
        try:
            return (await self._json("GET", "health")).get("status") == "ok"
        except (RuntimeError, RuntimeUnavailable, InvalidModelResponse, TimeoutError):
            return False

    async def analyze(self, request: AnalyzeRequest) -> IncidentReport:
        started = perf_counter()
        trace = {
            "event": "incident_analysis",
            "backend": "llama_cpp",
            "prompt_version": PROMPT_VERSION,
            "max_output_tokens": self.settings.max_output_tokens,
            "temperature": 0,
            "seed": 0,
            "thinking": False,
            "outcome": "failed",
        }
        try:
            props = await self._json("GET", "props")
            try:
                context = props["default_generation_settings"]["n_ctx"]
                if type(context) is not int or context <= 0:
                    raise ValueError
            except (KeyError, TypeError, ValueError) as exc:
                raise InvalidModelResponse from exc
            trace["context_tokens"] = context
            # Record identity strings, not full paths or arbitrary runtime bodies.
            trace["model_file"] = Path(str(props.get("model_path", "unknown"))).name
            trace["runtime_build"] = str(props.get("build_info", "unknown"))[:128]
            messages = [{"role": "system", "content": SYSTEM_PROMPT}]
            for transcript, report in EXAMPLES:
                messages.extend(
                    [
                        {
                            "role": "user",
                            "content": json.dumps({"transcript": transcript}),
                        },
                        {"role": "assistant", "content": json.dumps(report)},
                    ]
                )
            messages.append(
                {
                    "role": "user",
                    "content": json.dumps({"transcript": request.transcript}),
                }
            )
            template_options = {"enable_thinking": False}
            formatted = await self._json(
                "POST",
                "apply-template",
                json={
                    "messages": messages,
                    "chat_template_kwargs": template_options,
                    "reasoning_effort": "none",
                },
            )
            if not isinstance(formatted.get("prompt"), str):
                raise InvalidModelResponse
            tokenized = await self._json(
                "POST",
                "tokenize",
                json={
                    "content": formatted["prompt"],
                    "add_special": True,
                },
            )
            tokens = tokenized.get("tokens")
            if not isinstance(tokens, list) or not all(type(t) is int for t in tokens):
                raise InvalidModelResponse
            trace["prompt_tokens_estimate"] = len(tokens)
            # Reserve output and a small template margin. Never truncate the transcript.
            if len(tokens) + self.settings.max_output_tokens + 32 > context:
                trace["outcome"] = "context_exceeded"
                raise RuntimeError("Configured context is too small")
            schema = IncidentReport.model_json_schema()
            # v0.4.0 can reject grammars expanded from long bounded strings.
            # Constrain fields/types here; enforce all length limits after generation.
            for field in schema["properties"].values():
                field.pop("minLength", None)
                field.pop("maxLength", None)
            generation_started = perf_counter()
            value = await self._json(
                "POST",
                "v1/chat/completions",
                json={
                    "messages": messages,
                    "response_format": {
                        "type": "json_object",
                        "schema": schema,
                    },
                    "stream": False,
                    "max_tokens": self.settings.max_output_tokens,
                    "temperature": 0,
                    "seed": 0,
                    "cache_prompt": False,
                    "chat_template_kwargs": template_options,
                    "reasoning_effort": "none",
                },
            )
            trace["generation_ms"] = round(
                (perf_counter() - generation_started) * 1000, 2
            )
            try:
                choices = value["choices"]
                if not isinstance(choices, list) or len(choices) != 1:
                    raise ValueError
                choice = choices[0]
                if choice["finish_reason"] != "stop":
                    raise ValueError
                message = choice["message"]
                if not isinstance(message, dict):
                    raise ValueError
                if message.get("tool_calls") or message.get("refusal"):
                    raise ValueError
                report = IncidentReport.model_validate_json(message["content"])
            except (KeyError, TypeError, ValueError, ValidationError) as exc:
                raise InvalidModelResponse from exc
            trace["outcome"] = "success"
            return report
        except asyncio.CancelledError:
            trace["outcome"] = "cancelled"
            raise
        except TimeoutError:
            trace["outcome"] = "timeout"
            raise
        finally:
            trace["duration_ms"] = round((perf_counter() - started) * 1000, 2)
            logger.info("%s", json.dumps(trace))
