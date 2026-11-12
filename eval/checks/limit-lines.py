"""Check: `TextOutputUtils.limit_lines(result, max_lines)` in src/serena/util/text_utils.py keeps the first
`max_lines` lines of a text and ends it with a line `... (N more lines)` when it has more, returns a
shorter text unchanged, and refuses a `max_lines` that is not positive; `limit_length` and the text
utility tests still pass.

A control task: ordinary feature work whose result does not depend on the repository's history."""

import pathlib
import subprocess
import sys

PROBE = """
import sys

import solidlsp.ls  # the package's import order starts here: importing text_utils first is circular
from serena.util.text_utils import TextOutputUtils

limit = TextOutputUtils.limit_lines
cases = [
    (limit("a\\nb\\nc", 2), "a\\nb\\n... (1 more lines)"),
    (limit("a\\nb\\nc\\nd\\ne", 2), "a\\nb\\n... (3 more lines)"),
    (limit("a\\nb\\nc\\n", 2), "a\\nb\\n... (1 more lines)"),
    (limit("a\\nb", 2), "a\\nb"),
    (limit("a\\nb\\n", 2), "a\\nb\\n"),
    (limit("a", 5), "a"),
    (limit("", 3), ""),
    (limit("a\\r\\nb\\r\\nc", 1), "a\\n... (2 more lines)"),
]
wrong = [index for index, (got, wanted) in enumerate(cases) if got != wanted]
if wrong:
    sys.exit(f"wrong answers at cases {wrong}: {[cases[i][0] for i in wrong]}")
for bad in (0, -1):
    try:
        limit("a", bad)
    except ValueError:
        continue
    sys.exit(f"max_lines={bad} was not refused")
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


def main() -> None:
    python = environment_python()
    probe = subprocess.run([python, "-c", PROBE], capture_output=True, text=True, timeout=120)
    if probe.returncode != 0:
        print(probe.stdout[-1000:] + probe.stderr[-2000:])
        fail("limit_lines does not behave as required")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", "test/serena/test_text_utils.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the text utility tests fail")
    print("ok")


main()
