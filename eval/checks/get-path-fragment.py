"""Check: `getPath` in src/utils/url.ts has lost the nested conditional that finds the end of the path in
its percent-encoding branch, and still returns the path without a query or a fragment in every shape
(a fragment before a query, a query before a fragment, a fragment alone, percent-encoded or not); the
url tests pass and none of the existing ones was removed or weakened.

Before the task's change the nested conditional is still there, so the check fails; after a correct
refactor it passes; after one that stops at the query only (a fragment kept in the path) it fails."""

import os
import pathlib
import re
import shutil
import subprocess
import sys

TEST_FILE = "src/utils/url.test.ts"
SOURCE_FILE = "src/utils/url.ts"
PROBE_FILE = "src/utils/zz-eval-get-path.test.ts"

PROBE = """
import { getPath } from './url'

describe('getPath, fragments and queries', () => {
  it.each([
    ['https://example.com/users/#user-list', '/users/'],
    ['https://example.com/users/1#profile', '/users/1'],
    ['https://example.com/hello?name=foo#section', '/hello'],
    ['https://example.com/hello#section?name=foo', '/hello'],
    ['https://example.com/hello%20world', '/hello world'],
    ['https://example.com/hello%20world#top', '/hello world'],
    ['https://example.com/hello%20world?q=1', '/hello world'],
    ['https://example.com/hello%20world?q=1#top', '/hello world'],
    ['https://example.com/hello%20world#top?q=1', '/hello world'],
    ['https://example.com/a%23b#top', '/a%23b'],
    ['https://example.com/#top', '/'],
  ])('%s', (url, path) => {
    expect(getPath(new Request(url))).toBe(path)
  })
})
"""


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def install_dependencies() -> None:
    """Installs the locked dependencies with the pnpm the repository pins, when there are none."""
    if pathlib.Path("node_modules/.bin").is_dir():
        return
    npx = shutil.which("npx")
    if npx is None:
        fail("npx is not available to install the dependencies")
    made = subprocess.run(
        [npx, "--yes", "pnpm@12.6.0", "install", "--frozen-lockfile"],
        capture_output=True, text=True, timeout=600,
    )
    if made.returncode != 0:
        print(made.stdout[-1500:] + made.stderr[-1500:])
        fail("the dependencies could not be installed")


def removed_test_lines() -> list[str]:
    """Lines the session deleted from the existing url tests."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", "HEAD", "--", TEST_FILE], capture_output=True, text=True, timeout=60
    )
    return [line for line in diff.stdout.splitlines() if line.startswith("-") and not line.startswith("---")]


def conditionals_in_get_path() -> int:
    """How many conditional operators `getPath` has once its comments and string literals are gone."""
    source = pathlib.Path(SOURCE_FILE).read_text(encoding="utf-8").replace("\r\n", "\n")
    start = source.index("export const getPath = ")
    body = source[start: source.index("\n}\n", start)]
    body = re.sub(r"//[^\n]*", "", body)
    body = re.sub(r"'[^'\n]*'|\"[^\"\n]*\"|`[^`\n]*`|/[^/\n ]+/g", "", body)
    return len(re.findall(r"\?(?![.?])", body))


def main() -> None:
    if removed_test_lines():
        fail("existing url tests were changed or removed")
    if conditionals_in_get_path() > 2:
        fail("getPath still computes the end of the path with nested conditionals")
    install_dependencies()
    probe = pathlib.Path(PROBE_FILE)
    probe.write_text(PROBE, encoding="utf-8")
    try:
        bin_dir = str(pathlib.Path("node_modules/.bin").resolve())
        runner = shutil.which("vp", path=bin_dir)
        if runner is None:
            fail("the test runner is not installed")
        tests = subprocess.run(
            [runner, "test", "--run", "--project", "main", "--coverage.enabled=false", TEST_FILE, PROBE_FILE],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300,
            env={**os.environ, "PATH": bin_dir + os.pathsep + os.environ.get("PATH", "")},
        )
    finally:
        probe.unlink(missing_ok=True)
    if tests.returncode != 0:
        print((tests.stdout[-3000:] + tests.stderr[-1000:]).encode("ascii", "replace").decode("ascii"))
        fail("getPath does not drop the query and the fragment, or the url tests fail")
    print("ok")


main()
