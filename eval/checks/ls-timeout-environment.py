"""Check: SolidLanguageServer.create takes the request timeout it is not given from the environment
variable SOLIDLSP_REQUEST_TIMEOUT (seconds) when that is set; with the variable unset the default is
still the finite 300 seconds, never "no timeout"; a timeout passed to create wins over the variable,
and an explicit None still means no timeout; the solidlsp configuration and timeout unit tests, which
need no language server, pass.

Before the task's change the variable is ignored, so the check fails; after a correct change it passes;
after a change that makes "unset" mean unbounded it fails."""

import os
import pathlib
import subprocess
import sys

PROBE = """
import sys, tempfile
from types import SimpleNamespace

from solidlsp.ls import SolidLanguageServer

seen = []


class Stub(SolidLanguageServer):
    def __init__(self, config, repository_root_path, solidlsp_settings):
        pass

    def set_request_timeout(self, timeout):
        seen.append(timeout)


Stub.__abstractmethods__ = frozenset()
config = SimpleNamespace(ls_id=SimpleNamespace(get_ls_class=lambda: Stub))


def timeout_of(**kwargs):
    seen.clear()
    with tempfile.TemporaryDirectory() as root:
        SolidLanguageServer.create(config, root, **kwargs)
    return seen[-1]


scenario = sys.argv[1]
if scenario == "unset":
    expected = {"default": 300.0, "explicit": 42.0, "none": None}
else:
    expected = {"default": 900.0, "explicit": 42.0, "none": None}
got = {"default": timeout_of(), "explicit": timeout_of(timeout=42.0), "none": timeout_of(timeout=None)}
if got != expected:
    sys.exit(f"{scenario}: wanted {expected}, got {got}")
"""


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def environment_python() -> str:
    """The Python of the repository's environment, made with uv when the session made none."""
    for candidate in (".venv/Scripts/python.exe", ".venv/bin/python"):
        if pathlib.Path(candidate).is_file():
            return str(pathlib.Path(candidate).resolve())
    made = subprocess.run(
        ["uv", "sync", "--extra", "dev", "--python", "3.13", "--frozen"],
        capture_output=True, text=True, timeout=240,
    )
    if made.returncode != 0:
        print(made.stderr[-2000:])
        fail("the environment could not be made")
    return environment_python()


def run_probe(python: str, scenario: str) -> None:
    environment = {key: value for key, value in os.environ.items() if key != "SOLIDLSP_REQUEST_TIMEOUT"}
    if scenario == "set":
        environment["SOLIDLSP_REQUEST_TIMEOUT"] = "900"
    probe = subprocess.run(
        [python, "-c", PROBE, scenario], capture_output=True, text=True, timeout=120, env=environment
    )
    if probe.returncode != 0:
        print(probe.stdout[-1000:] + probe.stderr[-2000:])
        fail(f"the request timeout of SolidLanguageServer.create is not as required ({scenario})")


def main() -> None:
    python = environment_python()
    run_probe(python, "unset")
    run_probe(python, "set")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "test/solidlsp/test_ls_config.py", "test/solidlsp/test_typescript_timeout_policy.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the language server configuration and timeout tests fail")
    print("ok")


main()
