"""Exercise launcher argument boundaries without installing a model/runtime in CI."""

import json
import os
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "start-llama.sh"


@pytest.fixture
def environment(tmp_path):
    # Spaces and shell metacharacters must remain literal arguments, never execute.
    server = tmp_path / "fake runtime"
    server.write_text(
        "#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n"
    )
    server.chmod(0o755)
    model = tmp_path / "model $(touch injected).gguf"
    model.write_text("fixture, not model weights")
    env = {k: v for k, v in os.environ.items() if not k.startswith("VOICE_AGENT_")}
    env.update(
        VOICE_AGENT_LLAMA_SERVER_BIN=str(server), VOICE_AGENT_MODEL_PATH=str(model)
    )
    return env


def launch(env, *args):
    return subprocess.run(
        ["bash", str(SCRIPT), *args], env=env, capture_output=True, text=True, timeout=5
    )


def test_launch_forwards_literal_paths_and_settings(environment):
    environment.update(VOICE_AGENT_LLAMA_PORT="9001", VOICE_AGENT_LLAMA_GPU_LAYERS="12")
    result = launch(environment)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == [
        "--model",
        environment["VOICE_AGENT_MODEL_PATH"],
        "--host",
        "127.0.0.1",
        "--port",
        "9001",
        "--ctx-size",
        "4096",
        "--threads",
        "4",
        "--n-gpu-layers",
        "12",
    ]


def test_version_check_does_not_require_model(environment):
    environment.pop("VOICE_AGENT_MODEL_PATH")
    result = launch(environment, "--check")
    assert result.returncode == 0
    assert json.loads(result.stdout) == ["--version"]


def test_dry_run_does_not_execute_server(environment):
    server = Path(environment["VOICE_AGENT_LLAMA_SERVER_BIN"])
    server.write_text("#!/bin/sh\nexit 42\n")
    result = launch(environment, "--dry-run")
    assert result.returncode == 0
    assert "--model" in result.stdout
    assert "--ctx-size 4096" in result.stdout


@pytest.mark.parametrize(
    ("variable", "value"),
    [
        ("VOICE_AGENT_LLAMA_SERVER_BIN", "/missing/llama-server"),
        ("VOICE_AGENT_MODEL_PATH", "/missing/model.gguf"),
        ("VOICE_AGENT_MODEL_PATH", ""),
        ("VOICE_AGENT_LLAMA_PORT", "65536"),
        ("VOICE_AGENT_LLAMA_PORT", "0"),
        ("VOICE_AGENT_LLAMA_PORT", "999999999999999999999999999999"),
        ("VOICE_AGENT_LLAMA_CONTEXT", "-1"),
        ("VOICE_AGENT_LLAMA_THREADS", "$(touch injected)"),
        ("VOICE_AGENT_LLAMA_GPU_LAYERS", "1.5"),
        ("VOICE_AGENT_LLAMA_HOST", ""),
    ],
)
def test_bad_configuration_fails_before_launch(environment, variable, value):
    environment[variable] = value
    result = launch(environment)
    assert result.returncode != 0
    assert result.stderr.startswith("start-llama:")


def test_server_exit_status_is_preserved(environment):
    Path(environment["VOICE_AGENT_LLAMA_SERVER_BIN"]).write_text("#!/bin/sh\nexit 23\n")
    assert launch(environment).returncode == 23


def test_unknown_arguments_are_rejected(environment):
    assert launch(environment, "--unexpected").returncode != 0
    assert launch(environment, "--check", "--dry-run").returncode != 0
