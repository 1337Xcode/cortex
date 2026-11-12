"""Check: `serena project index` still writes the list of files it failed to index as UTF-8, whatever
the locale's encoding is (the test runs with text files opened without an encoding taken as cp1252, as
under a Windows locale); the project index tests pass, and the tests were not changed to get there.

Before the task's change the log is still written with open(), so the check fails; after a correct
refactor it passes; after a refactor that writes the log with the locale encoding it fails."""

import os
import pathlib
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
    source = pathlib.Path("src/serena/cli.py").read_text(encoding="utf-8")
    if "open(log_file" in source or "write_text(" not in source:
        fail("the failed-files log is still written by hand with open()")
    python = environment_python()
    environment = dict(os.environ)
    here = str(pathlib.Path(__file__).resolve().parent)
    environment["PYTHONPATH"] = here + os.pathsep + environment.get("PYTHONPATH", "")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", "-p", "eval_cp1252",
         "test/serena/test_cli_project_commands.py", "-k", "TestProjectIndex"],
        capture_output=True, text=True, timeout=600, env=environment,
    )
    if tests.returncode != 0:
        print(tests.stdout[-3000:])
        fail("the project command tests fail with a cp1252 locale")
    print("ok")


main()
