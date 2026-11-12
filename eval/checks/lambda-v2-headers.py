"""Check: `EventV2Processor.getHeaders` in src/adapter/aws-lambda/handler.ts builds its headers without a
hand-written loop (from the Headers constructor or a pipeline), and still answers an API Gateway v2
event whose header values are not ASCII (it encodes them, it does not throw), leaves out a header whose
value is missing or empty, and puts the cookies in a `cookie` header; the adapter tests pass and none
of the existing ones was removed or weakened.

Before the task's change the loop is still there, so the check fails; after a correct refactor it
passes; after one that hands the raw record to the Headers constructor it fails."""

import os
import pathlib
import re
import shutil
import subprocess
import sys

TEST_FILE = "src/adapter/aws-lambda/handler.test.ts"
SOURCE_FILE = "src/adapter/aws-lambda/handler.ts"
PROBE_FILE = "src/adapter/aws-lambda/zz-eval-v2-headers.test.ts"

PROBE = """
import { getProcessor } from './handler'
import type { LambdaEvent } from './handler'

const event = {
  version: '2.0',
  rawPath: '/my/path',
  rawQueryString: '',
  cookies: ['a=1', 'b=2'],
  headers: {
    host: 'example.com',
    'x-city': '\\u708e',
    'x-empty': '',
    'x-missing': undefined,
    'x-plain': 'value',
  },
  requestContext: { http: { method: 'GET', path: '/my/path', protocol: 'HTTP/1.1', sourceIp: '1.2.3.4', userAgent: 'a' } },
  isBase64Encoded: false,
} as unknown as LambdaEvent

describe('v2 header processing', () => {
  const request = getProcessor(event).createRequest(event)

  it('encodes non-ASCII values instead of throwing', () => {
    expect(decodeURIComponent(request.headers.get('x-city') ?? '')).toBe('\\u708e')
  })
  it('keeps plain values', () => {
    expect(request.headers.get('x-plain')).toBe('value')
  })
  it('leaves out headers without a value', () => {
    expect(request.headers.has('x-empty')).toBe(false)
    expect(request.headers.has('x-missing')).toBe(false)
  })
  it('joins the cookies', () => {
    expect(request.headers.get('cookie')).toBe('a=1; b=2')
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
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600,
    )
    if made.returncode != 0:
        print((made.stdout[-1500:] + made.stderr[-1500:]).encode("ascii", "replace").decode("ascii"))
        fail("the dependencies could not be installed")


def removed_test_lines() -> list[str]:
    """Lines the session deleted from the existing adapter tests."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", "HEAD", "--", TEST_FILE], capture_output=True, text=True, timeout=60
    )
    return [line for line in diff.stdout.splitlines() if line.startswith("-") and not line.startswith("---")]


def v2_get_headers_body() -> str:
    """The source of `EventV2Processor.getHeaders`, without comments."""
    source = pathlib.Path(SOURCE_FILE).read_text(encoding="utf-8").replace("\r\n", "\n")
    class_start = source.index("export class EventV2Processor")
    start = source.index("protected getHeaders(event: APIGatewayProxyEventV2)", class_start)
    body = source[start: source.index("\n  }\n", start)]
    return re.sub(r"//[^\n]*", "", body)


def main() -> None:
    if removed_test_lines():
        fail("existing adapter tests were changed or removed")
    body = v2_get_headers_body()
    if re.search(r"\bfor\s*\(", body) or ".forEach(" in body:
        fail("the v2 getHeaders still copies its headers with a hand-written loop")
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
        fail("the v2 event headers are not processed as required, or the adapter tests fail")
    print("ok")


main()
