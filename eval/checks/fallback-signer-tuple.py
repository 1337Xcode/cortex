"""Check: a serializer's fallback signers work in each of their three forms (an options dict, a
`(signer class, options)` tuple and a signer class) both when listed and when a value is loaded
through them; the tests of the signer, the serializer and the encoding pass, and the tests were not
changed to get there."""

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

    from itsdangerous.exc import BadSignature
    from itsdangerous.serializer import Serializer
    from itsdangerous.signer import Signer

    class Signer256(Signer):
        default_digest_method = hashlib.sha256

    signed = Serializer("secret", signer_kwargs={"digest_method": hashlib.sha256}).dumps("v")
    forms = {
        "an options dict": {"digest_method": hashlib.sha256},
        "a (signer class, options) tuple": (Signer, {"digest_method": hashlib.sha256}),
        "a signer class": Signer256,
    }
    for name, fallback in forms.items():
        serializer = Serializer("secret", fallback_signers=[fallback])
        unsigners = list(serializer.iter_unsigners())
        if len(unsigners) != 2 or unsigners[1].digest_method != hashlib.sha256:
            fail(f"{name} does not make a second unsigner with its own digest")
        if serializer.loads(signed) != "v":
            fail(f"{name} does not read a value signed with its digest")
    plain = Serializer("secret", fallback_signers=[(Signer, {"digest_method": hashlib.sha512})])
    try:
        plain.loads(signed)
    except BadSignature:
        pass
    else:
        fail("a fallback with a different digest accepted a signature it did not make")
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
