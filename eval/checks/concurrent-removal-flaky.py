"""Check: removing a project from the global configuration keeps the projects other configuration
instances added meanwhile and stays removed when reloaded, and adding one keeps the removals others
made; the configuration tests, which cover each of these, pass, and the tests were not changed to
get there."""

import pathlib
import re
import subprocess
import sys


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
    moved = subprocess.run(["git", "diff", "--quiet", "HEAD", "--", "test"], capture_output=True, timeout=60)
    if moved.returncode != 0:
        fail("the tests were changed")
    python = environment_python()
    named = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", "test/serena/config/test_serena_config.py",
         "-k", "concurrent or persists_across_reload or overlapping"],
        capture_output=True, text=True, timeout=240,
    )
    if named.returncode != 0 or not re.search(r"\b4 passed\b", named.stdout):
        print(named.stdout[-2000:])
        fail("the project list tests do not all pass")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", "test/serena/config/test_serena_config.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the configuration tests fail")
    print("ok")


main()
