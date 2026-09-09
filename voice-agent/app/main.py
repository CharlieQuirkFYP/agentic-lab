"""HTTP entry point. Run with uvicorn app.main:create_app --factory."""

import asyncio
import json
from contextlib import suppress

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from pydantic import ValidationError

from app.config import Settings
from app.incident_reporting.models import AnalyzeRequest, IncidentReport
from app.incident_reporting.service import (
    AnalysisService,
    Analyzer,
    InvalidModelResponse,
    RuntimeUnavailable,
    UnavailableAnalyzer,
)

ERRORS = {
    "invalid_request": (
        400,
        "A non-empty transcript of at most 16000 characters is required.",
    ),
    "unsupported_media_type": (415, "Content-Type must be application/json."),
    "runtime_unavailable": (503, "Analysis runtime is unavailable."),
    "analysis_timeout": (504, "Analysis timed out."),
    "invalid_model_response": (502, "Analysis runtime returned an invalid response."),
    "analysis_failed": (500, "Analysis failed."),
}


def error_response(code: str) -> JSONResponse:
    status, message = ERRORS[code]
    return JSONResponse(
        status_code=status, content={"error": {"code": code, "message": message}}
    )


async def analyze_until_disconnect(
    request: Request, service: AnalysisService, body: AnalyzeRequest
) -> IncidentReport:
    # The body has already been consumed. Listen for a disconnect while inference runs.
    async def disconnected() -> None:
        while True:
            if (await request.receive())["type"] == "http.disconnect":
                return

    analysis = asyncio.create_task(service.analyze(body))
    disconnect = asyncio.create_task(disconnected())
    try:
        await asyncio.wait({analysis, disconnect}, return_when=asyncio.FIRST_COMPLETED)
        if disconnect.done():
            raise asyncio.CancelledError
        return await analysis
    finally:
        for task in (analysis, disconnect):
            task.cancel()
        for task in (analysis, disconnect):
            with suppress(asyncio.CancelledError, Exception):
                await task


def create_app(
    settings: Settings | None = None, analyzer: Analyzer | None = None
) -> FastAPI:
    settings = settings if settings is not None else Settings.from_env()
    service = AnalysisService(
        analyzer if analyzer is not None else UnavailableAnalyzer(),
        settings.analysis_timeout_seconds,
    )
    app = FastAPI(title="Voice Agent", version="0.1.0")

    @app.get("/health")
    async def health() -> dict[str, str]:
        return {"status": "ok"}

    @app.get("/ready")
    async def ready() -> JSONResponse:
        if await service.is_ready():
            return JSONResponse({"status": "ready"})
        return error_response("runtime_unavailable")

    @app.post(
        "/v1/incidents/analyze",
        response_model=IncidentReport,
        openapi_extra={
            "requestBody": {
                "required": True,
                "content": {
                    "application/json": {"schema": AnalyzeRequest.model_json_schema()}
                },
            }
        },
    )
    async def analyze(request: Request):
        media_type = request.headers.get("content-type", "").split(";", 1)[0]
        if media_type.strip().lower() != "application/json":
            return error_response("unsupported_media_type")
        try:
            body = AnalyzeRequest.model_validate(await request.json())
        except (json.JSONDecodeError, UnicodeDecodeError, ValidationError):
            return error_response("invalid_request")
        try:
            return await analyze_until_disconnect(request, service, body)
        except RuntimeUnavailable:
            return error_response("runtime_unavailable")
        except TimeoutError:
            return error_response("analysis_timeout")
        except InvalidModelResponse:
            return error_response("invalid_model_response")
        except Exception:
            return error_response("analysis_failed")

    return app
