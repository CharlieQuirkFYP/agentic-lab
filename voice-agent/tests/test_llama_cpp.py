import asyncio
import json
import logging

import httpx
import pytest
from fastapi.testclient import TestClient

from app.adapters.llama_cpp import LlamaCppAnalyzer
from app.config import Settings
from app.incident_reporting.models import AnalyzeRequest
from app.incident_reporting.prompts import PROMPT_VERSION
from app.incident_reporting.service import (
    AnalysisService,
    InvalidModelResponse,
    RuntimeUnavailable,
)
from app.main import create_app

REPORT = {
    "incident_type": "smoke report",
    "location": "unknown",
    "severity": "unknown",
    "summary": "Smoke was reported.",
    "recommended_action": "unknown",
}


class Runtime:
    def __init__(self):
        self.requests = []
        self.overrides = {}
        self.context = 4096

    def handle(self, request):
        self.requests.append(request)
        path = request.url.path
        if path in self.overrides:
            result = self.overrides[path]
            if isinstance(result, Exception):
                raise result
            return result
        data = {
            "/health": {"status": "ok"},
            "/props": {
                "default_generation_settings": {"n_ctx": self.context},
                "model_path": "/private/models/test.gguf",
                "build_info": "test-commit",
            },
            "/apply-template": {"prompt": "formatted prompt"},
            "/tokenize": {"tokens": [1, 2, 3]},
            "/v1/chat/completions": {
                "choices": [
                    {
                        "finish_reason": "stop",
                        "message": {"content": json.dumps(REPORT)},
                    }
                ]
            },
        }
        return httpx.Response(200, json=data[path])


def run(runtime, operation="analyze", settings=None):
    async def scenario():
        async with httpx.AsyncClient(
            base_url="http://runtime/", transport=httpx.MockTransport(runtime.handle)
        ) as client:
            analyzer = LlamaCppAnalyzer(settings or Settings(), client)
            if operation == "ready":
                return await analyzer.is_ready()
            return await analyzer.analyze(
                AnalyzeRequest(transcript="private transcript")
            )

    return asyncio.run(scenario())


def test_extracts_schema_constrained_report_and_logs_safe_trace(caplog):
    runtime = Runtime()
    with caplog.at_level(logging.INFO, logger="uvicorn.error"):
        assert run(runtime).model_dump() == REPORT
    completion = json.loads(runtime.requests[-1].content)
    assert completion["stream"] is False
    assert completion["chat_template_kwargs"] == {"enable_thinking": False}
    assert completion["response_format"]["schema"]["additionalProperties"] is False
    assert (
        json.loads(completion["messages"][-1]["content"])["transcript"]
        == "private transcript"
    )
    trace = json.loads(caplog.records[-1].message)
    assert trace["outcome"] == "success"
    assert trace["prompt_version"] == PROMPT_VERSION
    assert trace["model_file"] == "test.gguf"
    assert trace["generation_ms"] >= 0
    assert "private transcript" not in caplog.text
    assert "/private/models" not in caplog.text


@pytest.mark.parametrize("status", [429, 502, 503])
def test_unavailable_runtime(status):
    runtime = Runtime()
    runtime.overrides["/props"] = httpx.Response(status, text="private failure")
    with pytest.raises(RuntimeUnavailable):
        run(runtime)
    assert len(runtime.requests) == 1


@pytest.mark.parametrize(
    "error", [httpx.ConnectError("private"), httpx.RemoteProtocolError("private")]
)
def test_connection_failure(error):
    runtime = Runtime()
    runtime.overrides["/props"] = error
    with pytest.raises(RuntimeUnavailable):
        run(runtime)


@pytest.mark.parametrize("error", [httpx.ReadTimeout("private"), httpx.Response(504)])
def test_runtime_timeout(error):
    runtime = Runtime()
    runtime.overrides["/props"] = error
    with pytest.raises(TimeoutError):
        run(runtime)


@pytest.mark.parametrize(
    "body",
    [
        {},
        {"choices": []},
        {"choices": [None]},
        {
            "choices": [
                {"finish_reason": "length", "message": {"content": json.dumps(REPORT)}}
            ]
        },
        {"choices": [{"finish_reason": "stop", "message": {"content": "not JSON"}}]},
        {"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]},
        {"choices": [{"finish_reason": "stop", "message": {"content": None}}]},
        {
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {
                        "content": json.dumps({**REPORT, "severity": "critical"})
                    },
                }
            ]
        },
    ],
)
def test_rejects_invalid_or_truncated_completions(body):
    runtime = Runtime()
    runtime.overrides["/v1/chat/completions"] = httpx.Response(200, json=body)
    with pytest.raises(InvalidModelResponse):
        run(runtime)
    assert (
        len([r for r in runtime.requests if r.url.path == "/v1/chat/completions"]) == 1
    )


@pytest.mark.parametrize(
    "path,body",
    [
        ("/props", {}),
        ("/props", {"default_generation_settings": {"n_ctx": True}}),
        ("/apply-template", {"prompt": None}),
        ("/tokenize", {"tokens": "invalid"}),
    ],
)
def test_rejects_bad_preflight_data(path, body):
    runtime = Runtime()
    runtime.overrides[path] = httpx.Response(200, json=body)
    with pytest.raises(InvalidModelResponse):
        run(runtime)
    assert not any(r.url.path == "/v1/chat/completions" for r in runtime.requests)


def test_context_overflow_never_generates_or_truncates():
    runtime = Runtime()
    runtime.context = 512
    with pytest.raises(RuntimeError, match="context"):
        run(runtime)
    assert not any(r.url.path == "/v1/chat/completions" for r in runtime.requests)


def test_readiness_tracks_runtime_state():
    runtime = Runtime()
    assert run(runtime, "ready") is True
    for result in [
        httpx.Response(503),
        httpx.Response(200, json={"status": "loading model"}),
        httpx.Response(200, text="invalid"),
        httpx.ConnectError("offline"),
    ]:
        runtime.overrides["/health"] = result
        assert run(runtime, "ready") is False


def test_service_timeout_cancels_http_transport():
    async def scenario():
        cancelled = asyncio.Event()

        async def blocked(request):
            try:
                await asyncio.Event().wait()
            finally:
                cancelled.set()

        async with httpx.AsyncClient(
            base_url="http://runtime/", transport=httpx.MockTransport(blocked)
        ) as client:
            analyzer = LlamaCppAnalyzer(Settings(), client)
            service = AnalysisService(analyzer, 0.01)
            with pytest.raises(TimeoutError):
                await service.analyze(AnalyzeRequest(transcript="smoke"))
            assert cancelled.is_set()

    asyncio.run(scenario())


def test_factory_wires_adapter_and_closes_owned_client(monkeypatch):
    runtime = Runtime()
    adapter = LlamaCppAnalyzer(Settings())
    # Replace only the transport while retaining ownership/lifecycle behaviour.
    asyncio.run(adapter.client.aclose())
    adapter.client = httpx.AsyncClient(
        base_url="http://runtime/", transport=httpx.MockTransport(runtime.handle)
    )
    monkeypatch.setattr("app.main.LlamaCppAnalyzer", lambda settings: adapter)
    with TestClient(create_app(Settings(analyzer_backend="llama_cpp"))) as client:
        assert client.get("/ready").status_code == 200
        response = client.post("/v1/incidents/analyze", json={"transcript": "Smoke."})
        assert response.status_code == 200
        assert response.json() == REPORT
    assert adapter.client.is_closed


@pytest.mark.parametrize(
    "report",
    [
        {**REPORT, "summary": "x" * 2001},
        {**REPORT, "location": "x" * 513},
        {**REPORT, "unexpected": "private"},
    ],
)
def test_full_python_schema_applies_after_relaxed_generation_schema(report):
    runtime = Runtime()
    runtime.overrides["/v1/chat/completions"] = httpx.Response(
        200,
        json={
            "choices": [
                {"finish_reason": "stop", "message": {"content": json.dumps(report)}}
            ],
        },
    )
    with pytest.raises(InvalidModelResponse):
        run(runtime)
    schema = json.loads(runtime.requests[-1].content)["response_format"]["schema"]
    assert "maxLength" not in schema["properties"]["summary"]
    assert schema["additionalProperties"] is False


@pytest.mark.parametrize("body", [b"not JSON", b"[]"])
def test_malformed_runtime_envelope(body):
    runtime = Runtime()
    runtime.overrides["/props"] = httpx.Response(200, content=body)
    with pytest.raises(InvalidModelResponse):
        run(runtime)
