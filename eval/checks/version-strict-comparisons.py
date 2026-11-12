"""Check: `Version` in src/serena/util/version.py has `is_greater_than` and `is_less_than`, which compare
exactly the components given (a version equal on all of them is neither), and the existing comparisons
still behave as before.

A control task: ordinary feature work whose result does not depend on the repository's history."""

import pathlib
import subprocess
import sys

PROBE = """
import sys

from serena.util.version import Version

v = Version("1.2.3")
cases = [
    (v.is_greater_than(1, 2, 2), True), (v.is_greater_than(1, 2, 3), False), (v.is_greater_than(1, 2, 4), False),
    (v.is_greater_than(0, 9, 9), True), (v.is_greater_than(2, 0, 0), False), (v.is_greater_than(1, 2), False),
    (v.is_greater_than(1, 1), True), (v.is_greater_than(1), False), (v.is_greater_than(0), True),
    (v.is_less_than(1, 2, 4), True), (v.is_less_than(1, 2, 3), False), (v.is_less_than(1, 2, 2), False),
    (v.is_less_than(2, 0, 0), True), (v.is_less_than(0, 9, 9), False), (v.is_less_than(1, 2), False),
    (v.is_less_than(1, 3), True), (v.is_less_than(1), False), (v.is_less_than(2), True),
    (Version("2.0.0rc1").is_greater_than(1, 9, 9), True), (Version("2.0.0rc1").is_less_than(2, 0, 1), True),
    (v.is_at_least(1, 2, 3), True), (v.is_at_most(1, 2, 3), True), (v.is_equal(1, 2), True), (v.is_equal(1, 3), False),
]
wrong = [index for index, (got, wanted) in enumerate(cases) if got is not wanted]
if wrong:
    sys.exit(f"wrong answers at cases {wrong}")
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
        fail("Version does not compare as required")
    print("ok")


main()
