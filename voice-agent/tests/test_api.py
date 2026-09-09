import asyncio

import pytest
from fastapi.testclient import TestClient

from app.config import Settings
from app.incident_reporting.service import RuntimeUnavailable
from app.main import create_app

REPORT = {
    "incident_type": "collision",
    "location": "west entrance",
    "severity": "unknown",
    "summary": "A vehicle collided with a barrier near the west entrance.",
    "recommended_action": "unknown",
}
PATH = "/v1/incidents/analyze"


class FakeAnalyzer:
    def __init__(self, result=None, error=None):
        self.result = REPORT if result is None else result
        self.error = error
        self.requests = []

    async def analyze(self, request):
        self.requests.append(request)
        if self.error:
            raise self.error
        return self.result

    async def is_ready(self):
        return True


def test_default_service_is_live_but_not_ready():
    with TestClient(create_app(Settings())) as client:
        assert client.get("/health").json() == {"status": "ok"}
        assert client.get("/ready").status_code == 503
        response = client.post(PATH, json={"transcript": "Smoke reported."})
        assert response.status_code == 503
        assert response.json() == {
            "error": {
                "code": "runtime_unavailable",
                "message": "Analysis runtime is unavailable.",
            }
        }


def test_success_preserves_contract_and_normalizes_input():
    analyzer = FakeAnalyzer()
    with TestClient(create_app(Settings(), analyzer)) as client:
        assert client.get("/ready").json() == {"status": "ready"}
        response = client.post(
            PATH,
            content='{"transcript":"  Collision at west entrance.  "}',
            headers={"content-type": "application/json; charset=utf-8"},
        )
        assert response.status_code == 200
        assert response.json() == REPORT
        assert analyzer.requests[0].transcript == "Collision at west entrance."


@pytest.mark.parametrize(
    "payload",
    [
        {},
        [],
        None,
        "text",
        {"transcript": None},
        {"transcript": 12},
        {"transcript": True},
        {"transcript": []},
        {"transcript": ""},
        {"transcript": " \n\t"},
        {"transcript": "x" * 16001},
        {"transcript": "smoke", "extra": "secret"},
    ],
)
def test_invalid_requests_never_reach_analyzer(payload):
    import json

    analyzer = FakeAnalyzer()
    with TestClient(create_app(Settings(), analyzer)) as client:
        response = client.post(
            PATH,
            content=json.dumps(payload),
            headers={"content-type": "application/json"},
        )
        assert response.status_code == 400
        assert response.json()["error"]["code"] == "invalid_request"
        assert not analyzer.requests


@pytest.mark.parametrize("content", [b'{"transcript":', b"\xff", b""])
def test_malformed_json(content):
    with TestClient(create_app(Settings())) as client:
        response = client.post(
            PATH, content=content, headers={"content-type": "application/json"}
        )
        assert response.status_code == 400


@pytest.mark.parametrize("media_type", [None, "text/plain", "application/problem+json"])
def test_media_type_is_required(media_type):
    with TestClient(create_app(Settings())) as client:
        headers = {} if media_type is None else {"content-type": media_type}
        response = client.post(PATH, content='{"transcript":"smoke"}', headers=headers)
        assert response.status_code == 415


def test_character_limit_is_unicode_characters_after_trimming():
    analyzer = FakeAnalyzer()
    with TestClient(create_app(Settings(), analyzer)) as client:
        response = client.post(PATH, json={"transcript": "  " + "声" * 16000 + "  "})
        assert response.status_code == 200
        assert len(analyzer.requests[0].transcript) == 16000


@pytest.mark.parametrize(
    "result",
    [
        {},
        {**REPORT, "severity": "critical"},
        {**REPORT, "location": 1},
        {**REPORT, "summary": " "},
        {**REPORT, "summary": "x" * 2001},
        {**REPORT, "extra": "private model output"},
        [REPORT],
    ],
)
def test_invalid_model_output_is_not_exposed(result):
    with TestClient(create_app(Settings(), FakeAnalyzer(result=result))) as client:
        response = client.post(PATH, json={"transcript": "smoke"})
        assert response.status_code == 502
        assert response.json()["error"]["code"] == "invalid_model_response"
        assert "private" not in response.text


@pytest.mark.parametrize(
    ("error", "status", "code"),
    [
        (RuntimeUnavailable("secret"), 503, "runtime_unavailable"),
        (RuntimeError("secret transcript and path"), 500, "analysis_failed"),
    ],
)
def test_errors_are_sanitized(error, status, code):
    with TestClient(create_app(Settings(), FakeAnalyzer(error=error))) as client:
        response = client.post(PATH, json={"transcript": "smoke"})
        assert response.status_code == status
        assert response.json()["error"]["code"] == code
        assert "secret" not in response.text


class BlockingAnalyzer(FakeAnalyzer):
    cancelled = False

    async def analyze(self, request):
        try:
            await asyncio.Event().wait()
        finally:
            self.cancelled = True


def test_timeout_cancels_analyzer_without_retry():
    analyzer = BlockingAnalyzer()
    with TestClient(
        create_app(Settings(analysis_timeout_seconds=0.01), analyzer)
    ) as client:
        response = client.post(PATH, json={"transcript": "smoke"})
        assert response.status_code == 504
        assert response.json()["error"]["code"] == "analysis_timeout"
        assert analyzer.cancelled


def test_disconnect_cancels_inflight_analysis():
    async def scenario():
        from starlette.requests import Request

        from app.incident_reporting.models import AnalyzeRequest
        from app.incident_reporting.service import AnalysisService
        from app.main import analyze_until_disconnect

        started = asyncio.Event()
        analyzer = BlockingAnalyzer()
        original = analyzer.analyze

        async def analyze(body):
            started.set()
            return await original(body)

        analyzer.analyze = analyze

        async def receive():
            await started.wait()
            return {"type": "http.disconnect"}

        request = Request({"type": "http"}, receive)
        with pytest.raises(asyncio.CancelledError):
            await analyze_until_disconnect(
                request,
                AnalysisService(analyzer, 10),
                AnalyzeRequest(transcript="smoke"),
            )
        assert analyzer.cancelled

    asyncio.run(scenario())
