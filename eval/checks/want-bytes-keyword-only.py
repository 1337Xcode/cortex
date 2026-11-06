"""Check: `want_bytes` takes its encoding and error handling only by keyword; every caller in the
package still works, as the tests of encoding, the signer and the serializer show."""

import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path("src").resolve()))


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    from itsdangerous.encoding import base64_decode, base64_encode, want_bytes

    try:
        want_bytes("value", "ascii")
    except TypeError:
        pass
    else:
        fail("a positional encoding is still accepted")
    try:
        want_bytes("value", encoding="ascii", errors="strict", extra=1)  # type: ignore[call-arg]
    except TypeError:
        pass
    else:
        fail("want_bytes accepts any keyword")
    if want_bytes("é", encoding="ascii", errors="replace") != b"?":
        fail("the keywords no longer reach the encoding")
    if want_bytes(b"ab") != b"ab" or want_bytes("ab") != b"ab":
        fail("want_bytes no longer passes bytes through or encodes text as UTF-8")
    if base64_decode(base64_encode("value")) != b"value":
        fail("base64 no longer round-trips")
    tests = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "tests/test_itsdangerous/test_encoding.py", "tests/test_itsdangerous/test_signer.py",
         "tests/test_itsdangerous/test_serializer.py"],
        env={"PYTHONPATH": "src", "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
             "PATH": os.environ.get("PATH", "")},
        capture_output=True, text=True, timeout=300,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the package's tests fail")
    print("ok")


main()
