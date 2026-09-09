"""Analysis orchestration, independent of HTTP and any model runtime."""

import asyncio
from typing import Protocol

from pydantic import ValidationError

from app.incident_reporting.models import AnalyzeRequest, IncidentReport


class RuntimeUnavailable(Exception):
    pass


class InvalidModelResponse(Exception):
    pass


class Analyzer(Protocol):
    async def analyze(self, request: AnalyzeRequest) -> object: ...

    async def is_ready(self) -> bool: ...


class UnavailableAnalyzer:
    """Explicit placeholder until a real runtime adapter is implemented."""

    async def analyze(self, request: AnalyzeRequest) -> object:
        raise RuntimeUnavailable

    async def is_ready(self) -> bool:
        return False


class AnalysisService:
    def __init__(self, analyzer: Analyzer, timeout_seconds: float):
        self.analyzer = analyzer
        self.timeout_seconds = timeout_seconds

    async def analyze(self, request: AnalyzeRequest) -> IncidentReport:
        async with asyncio.timeout(self.timeout_seconds):
            result = await self.analyzer.analyze(request)
            try:
                return IncidentReport.model_validate(result)
            except ValidationError as exc:
                raise InvalidModelResponse from exc

    async def is_ready(self) -> bool:
        try:
            async with asyncio.timeout(self.timeout_seconds):
                return await self.analyzer.is_ready()
        except Exception:
            return False
