"""The initial stateless text-analysis contract."""

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, StringConstraints, field_validator


class ContractModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", str_strip_whitespace=True)


class AnalyzeRequest(ContractModel):
    transcript: Annotated[str, StringConstraints(min_length=1, max_length=16000)]


class IncidentReport(ContractModel):
    incident_type: Annotated[str, StringConstraints(min_length=1, max_length=128)]
    location: Annotated[str, StringConstraints(min_length=1, max_length=512)]
    severity: Literal["low", "medium", "high", "unknown"]
    summary: Annotated[str, StringConstraints(min_length=1, max_length=2000)]
    recommended_action: Annotated[str, StringConstraints(min_length=1, max_length=512)]

    @field_validator("severity", mode="before")
    @classmethod
    def strip_severity(cls, value: object) -> object:
        return value.strip() if isinstance(value, str) else value
