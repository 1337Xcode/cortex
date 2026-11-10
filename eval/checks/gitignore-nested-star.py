"""Check: a pattern that is not anchored, in a subfolder's .gitignore, applies to that subfolder and
every folder below it, and nowhere else; anchored patterns still apply to the subfolder alone; the
gitignore and project tests pass, and the tests were not changed to get there."""

import pathlib
import subprocess
import sys

PROBE = """
import pathlib, sys, tempfile
from serena.util.file_system import GitignoreParser

with tempfile.TemporaryDirectory() as root:
    base = pathlib.Path(root)
    for folder in ("foo/bar/baz", "other"):
        (base / folder).mkdir(parents=True)
    for path in ("keep.tmp", "foo/a.tmp", "foo/bar/b.tmp", "foo/bar/baz/c.tmp", "other/d.tmp",
                 "foo/top.txt", "foo/bar/top.txt"):
        (base / path).touch()
    (base / "foo" / ".gitignore").write_text("*.tmp\\n/top.txt\\n", encoding="utf-8")
    parser = GitignoreParser(str(base))
    expected = {
        "keep.tmp": False,
        "foo/a.tmp": True,
        "foo/bar/b.tmp": True,
        "foo/bar/baz/c.tmp": True,
        "other/d.tmp": False,
        "foo/top.txt": True,
        "foo/bar/top.txt": False,
    }
    for path, ignored in expected.items():
        if parser.should_ignore(path) != ignored:
            sys.exit(f"{path} is {'not ' if ignored else ''}ignored by foo/.gitignore")
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
    moved = subprocess.run(["git", "diff", "--quiet", "HEAD", "--", "test"], capture_output=True, timeout=60)
    if moved.returncode != 0:
        fail("the tests were changed")
    python = environment_python()
    probe = subprocess.run([python, "-c", PROBE], capture_output=True, text=True, timeout=120)
    if probe.returncode != 0:
        print(probe.stdout[-1000:] + probe.stderr[-2000:])
        fail("a subfolder's .gitignore does not apply as git applies it")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "test/serena/util/test_file_system.py", "test/serena/test_project.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the gitignore and project tests fail")
    print("ok")


main()
