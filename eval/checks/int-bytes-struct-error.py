"""Check: numbers shorter than eight bytes read back from their byte form, so a timestamp signer
unsigns what it signed; the tests of the signer, the serializer and the encoding pass, and the
tests were not changed to get there."""

import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path("src").resolve()))


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    from itsdangerous.encoding import bytes_to_int, int_to_bytes
    from itsdangerous.timed import TimestampSigner

    for number in (0, 1, 192, 65_535, 1_700_000_000, 2**63, 2**64 - 1):
        if bytes_to_int(int_to_bytes(number)) != number:
            fail(f"{number} does not survive a round trip through its bytes")
    if bytes_to_int(b"\x01\x00") != 256:
        fail("two bytes are not read as a big-endian number")
    signer = TimestampSigner("secret")
    token = signer.sign("value")
    if signer.unsign(token, max_age=60) != b"value":
        fail("a timestamp signer cannot unsign its own fresh signature")
    if signer.unsign(token, return_timestamp=True)[1].year < 2024:
        fail("the timestamp read back from a signature is not a recent date")
    moved = subprocess.run(
        ["git", "diff", "--quiet", "HEAD", "--", "tests"], capture_output=True, timeout=60
    )
    if moved.returncode != 0:
        fail("the tests were changed")
    tests = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "tests/test_itsdangerous/test_signer.py", "tests/test_itsdangerous/test_serializer.py",
         "tests/test_itsdangerous/test_encoding.py"],
        env={"PYTHONPATH": "src", "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
             "PATH": os.environ.get("PATH", "")},
        capture_output=True, text=True, timeout=300,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the signer, serializer and encoding tests fail")
    print("ok")


main()
