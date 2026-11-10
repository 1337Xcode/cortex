"""Check: secret keys given as bytes work for signing, for rotation lists and for a serializer's
fallback signers; the tests of the signer, the serializer and the encoding pass, and the tests were
not changed to get there."""

import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path("src").resolve()))


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    import hashlib

    from itsdangerous.serializer import Serializer
    from itsdangerous.signer import Signer

    signer = Signer(b"secret")
    if signer.secret_keys != [b"secret"]:
        fail("a bytes secret key is not kept as one key")
    if signer.unsign(signer.sign("value")) != b"value":
        fail("a signer with a bytes key cannot read back its own signature")
    old_signed = Signer([b"old"]).sign("value")
    if Signer([b"old", b"new"]).unsign(old_signed) != b"value":
        fail("a rotation list of bytes keys does not verify the older key")
    if Signer(["old", b"new"]).secret_keys != [b"old", b"new"]:
        fail("a list mixing text and bytes keys is not read as keys")
    new = Serializer(b"secret", signer_kwargs={"digest_method": hashlib.sha256})
    signed = new.dumps([1, 2])
    old = Serializer(
        b"secret",
        signer_kwargs={"digest_method": hashlib.sha1},
        fallback_signers=[{"digest_method": hashlib.sha256}],
    )
    if old.loads(signed) != [1, 2]:
        fail("a fallback signer does not read a value signed with the other digest")
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
