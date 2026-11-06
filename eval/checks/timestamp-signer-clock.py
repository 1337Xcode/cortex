"""Check: `TimestampSigner` takes an optional `clock`, which signing and the age check both follow,
`time.time` by default; a timed serializer passes it through its signer options; the tests of the
signer and the serializer still pass."""

import os
import pathlib
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path("src").resolve()))


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    from itsdangerous.exc import SignatureExpired
    from itsdangerous.timed import TimedSerializer, TimestampSigner

    early = TimestampSigner("secret", clock=lambda: 1_000_000)
    if early.get_timestamp() != 1_000_000:
        fail("get_timestamp does not follow the clock")
    token = early.sign("value")
    late = TimestampSigner("secret", clock=lambda: 1_000_010)
    if late.unsign(token, max_age=20) != b"value":
        fail("a signature 10 s old is refused with max_age 20")
    try:
        late.unsign(token, max_age=5)
    except SignatureExpired:
        pass
    else:
        fail("a signature 10 s old is accepted with max_age 5")
    if abs(TimestampSigner("secret").get_timestamp() - time.time()) > 5:
        fail("without a clock the timestamp is not the current time")
    signer = TimestampSigner("secret", salt="s", key_derivation="hmac", clock=lambda: 7)
    if signer.key_derivation != "hmac" or signer.salt != b"s":
        fail("the signer's other options are lost")
    serializer = TimedSerializer("secret", signer_kwargs={"clock": lambda: 2_000_000})
    signed = serializer.dumps({"a": 1})
    if serializer.loads(signed, max_age=1) != {"a": 1}:
        fail("a timed serializer does not sign with the clock it was given")
    tests = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "tests/test_itsdangerous/test_signer.py", "tests/test_itsdangerous/test_serializer.py"],
        env={"PYTHONPATH": "src", "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
             "PATH": os.environ.get("PATH", "")},
        capture_output=True, text=True, timeout=300,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the signer and serializer tests fail")
    print("ok")


main()
