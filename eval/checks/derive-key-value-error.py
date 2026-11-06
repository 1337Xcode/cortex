"""Check: `Signer` refuses an unknown key derivation at construction, and when deriving a key, with
a `ValueError` naming the method; the tests of the signer and the serializer still pass."""

import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path("src").resolve()))


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    from itsdangerous.signer import Signer

    for known in ("concat", "django-concat", "hmac", "none"):
        signer = Signer("secret", key_derivation=known)
        if signer.unsign(signer.sign("value")) != b"value":
            fail(f"signing with {known} no longer round-trips")
    try:
        Signer("secret", key_derivation="md5")
    except ValueError as error:
        if "md5" not in str(error):
            fail("the constructor's ValueError does not name the method it was given")
    except Exception as error:  # noqa: BLE001 - any other error is the wrong one
        fail(f"the constructor raised {type(error).__name__}, not ValueError")
    else:
        fail("the constructor accepted an unknown key derivation")
    signer = Signer("secret")
    signer.key_derivation = "bogus"
    try:
        signer.derive_key()
    except ValueError as error:
        if "bogus" not in str(error):
            fail("derive_key's ValueError does not name the method")
    except Exception as error:  # noqa: BLE001 - any other error is the wrong one
        fail(f"derive_key raised {type(error).__name__}, not ValueError")
    else:
        fail("derive_key accepted an unknown key derivation")
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
