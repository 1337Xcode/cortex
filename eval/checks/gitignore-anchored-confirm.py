"""Check: serena already treats an anchored pattern in a subfolder's .gitignore as git does (`/build`
in `src/.gitignore` ignores `src/build` and not `src/lib/build`), so the right answer changes
nothing: no file under src or test was changed or added, the behaviour still holds, and the
gitignore tests pass."""

import pathlib
import subprocess
import sys

PROBE = """
import pathlib, sys, tempfile
from serena.util.file_system import GitignoreParser

with tempfile.TemporaryDirectory() as root:
    base = pathlib.Path(root)
    for folder in ("src/build", "src/lib/build"):
        (base / folder).mkdir(parents=True)
        (base / folder / "out.o").touch()
    (base / "src" / ".gitignore").write_text("/build\\n", encoding="utf-8")
    parser = GitignoreParser(str(base))
    if not parser.should_ignore("src/build/out.o"):
        sys.exit("src/build is no longer ignored by /build in src/.gitignore")
    if parser.should_ignore("src/lib/build/out.o"):
        sys.exit("src/lib/build is ignored by /build in src/.gitignore")
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
    changed = subprocess.run(
        ["git", "status", "--porcelain", "--", "src", "test"],
        capture_output=True, text=True, timeout=60,
    )
    if changed.returncode != 0 or changed.stdout.strip():
        print(changed.stdout[-2000:])
        fail("files under src or test were changed, though serena already behaves like git here")
    python = environment_python()
    probe = subprocess.run([python, "-c", PROBE], capture_output=True, text=True, timeout=120)
    if probe.returncode != 0:
        print(probe.stdout[-1000:] + probe.stderr[-2000:])
        fail("the anchored pattern does not behave as git's")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider", "test/serena/util/test_file_system.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the gitignore tests fail")
    print("ok")


main()
