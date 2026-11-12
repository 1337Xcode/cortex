"""Check: the cpp setting `max_references` reaches clangd as `--limit-references=<n>`, exactly once; with
no setting clangd still returns every reference (`--limit-references=0`), and a user's own
`--limit-references` in `ls_extra_args` is still the only one; the clangd launch tests pass and none of
the existing ones was removed or weakened.

Before the task's change the setting is unknown, so the check fails; after a correct change it passes;
after a change whose default is clangd's own limit (no flag, or a number) it fails."""

import pathlib
import subprocess
import sys

TEST_FILE = "test/solidlsp/cpp/test_clangd_dependency_provider.py"

PROBE = """
import sys, tempfile

from solidlsp.language_servers.clangd_language_server import ClangdLanguageServer
from solidlsp.settings import SolidLSPSettings


def command(settings):
    with tempfile.TemporaryDirectory() as resources:
        provider = ClangdLanguageServer.DependencyProvider(
            SolidLSPSettings.CustomLSSettings({"ls_path": "/opt/clangd", **settings}), resources
        )
        return provider.create_launch_command()


def limits(settings):
    return [arg for arg in command(settings) if arg.lstrip("-").split("=", 1)[0] == "limit-references"]


def expect(settings, wanted, why):
    got = limits(settings)
    if got != wanted:
        sys.exit(f"{why}: wanted {wanted}, got {got}")


expect({}, ["--limit-references=0"], "no setting must still return every reference")
expect({"max_references": 200}, ["--limit-references=200"], "max_references must reach clangd")
expect({"max_references": 5000}, ["--limit-references=5000"], "max_references must reach clangd")
expect({"ls_extra_args": ["--limit-references=5"]}, ["--limit-references=5"], "the user's own limit must stay the only one")
expect({"ls_extra_args": ["-limit-references=5"]}, ["-limit-references=5"], "the user's own limit must stay the only one")
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


def removed_test_lines() -> list[str]:
    """Lines the session deleted from the existing launch tests."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", "HEAD", "--", TEST_FILE], capture_output=True, text=True, timeout=60
    )
    return [line for line in diff.stdout.splitlines() if line.startswith("-") and not line.startswith("---")]


def main() -> None:
    if removed_test_lines():
        fail("existing launch tests were changed or removed")
    python = environment_python()
    probe = subprocess.run([python, "-c", PROBE], capture_output=True, text=True, timeout=120)
    if probe.returncode != 0:
        print(probe.stdout[-1000:] + probe.stderr[-2000:])
        fail("the clangd launch command is not as required")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", TEST_FILE],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the clangd launch tests fail")
    print("ok")


main()
