"""Check: the gitignore walker still skips a directory it may not read (a PermissionError from
os.scandir) and carries on with its siblings, and every directory listing it opens is closed by the
walker itself (a `with` block or an explicit close), not left to the garbage collector; the gitignore
and project tests pass, and the tests were not changed to get there.

Before the task's change the first half holds and the second does not, so the check fails; after a
correct change it passes; after a change that drops the permission guard it fails again."""

import pathlib
import subprocess
import sys

PROBE = """
import os, pathlib, sys, tempfile

from serena.util.file_system import GitignoreParser

opened = []
real_scandir = os.scandir


class Tracked:
    \"\"\"A directory listing that remembers whether its owner closed it.\"\"\"

    def __init__(self, inner):
        self.inner = inner
        self.closed = False

    def __iter__(self):
        return iter(self.inner)

    def __next__(self):
        return next(self.inner)

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        self.closed = True
        self.inner.close()
        return False

    def close(self):
        self.closed = True
        self.inner.close()


with tempfile.TemporaryDirectory() as root:
    base = pathlib.Path(root)
    (base / "locked").mkdir()
    (base / "open").mkdir()
    (base / ".gitignore").write_text("*.log\\n", encoding="utf-8")
    (base / "open" / ".gitignore").write_text("*.tmp\\n", encoding="utf-8")
    locked = os.path.abspath(str(base / "locked"))

    def scandir(path="."):
        if os.path.abspath(os.fspath(path)) == locked:
            raise PermissionError(13, "Permission denied", os.fspath(path))
        listing = Tracked(real_scandir(path))
        opened.append(listing)
        return listing

    os.scandir = scandir
    try:
        try:
            parser = GitignoreParser(str(base))
        except PermissionError:
            sys.exit("GUARD: an unreadable directory made the gitignore walker fail")
    finally:
        os.scandir = real_scandir

    if not parser.should_ignore("a.log") or not parser.should_ignore("open/b.tmp"):
        sys.exit("GUARD: the walker skipped the readable directories after the unreadable one")
    if not opened:
        sys.exit("PROBE: the walker did not list any directory through os.scandir")
    if not all(listing.closed for listing in opened):
        sys.exit("CLOSE: the walker leaves directory listings open")
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
        fail("the gitignore walker's directory handling is not as required")
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
