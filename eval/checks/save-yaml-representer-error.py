"""Check: saving YAML keeps a loaded file's comments by default and still saves a plain mapping when
asked not to keep them, so Serena's configuration can be saved again; the YAML and configuration
tests pass, and the tests were not changed to get there."""

import pathlib
import subprocess
import sys

PROBE = """
import pathlib, sys, tempfile
from serena.util.yaml import load_yaml, save_yaml

with tempfile.TemporaryDirectory() as root:
    base = pathlib.Path(root)
    path = base / "config.yml"
    path.write_text("# the header\\nkey: 1  # the note\\n", encoding="utf-8")
    data = load_yaml(str(path))
    data["key"] = 2
    save_yaml(str(path), data)
    text = path.read_text(encoding="utf-8")
    if "# the header" not in text or "# the note" not in text or "key: 2" not in text:
        sys.exit("a loaded file saved with the default does not keep its comments: " + text)
    plain = base / "plain.yml"
    save_yaml(str(plain), {"a": [1, 2]}, preserve_comments=False)
    if dict(load_yaml(str(plain))) != {"a": [1, 2]}:
        sys.exit("a plain mapping saved without comments does not read back")
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
        fail("saving YAML does not behave as it should")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "test/serena/util/test_yaml.py", "test/serena/config/test_serena_config.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the YAML and configuration tests fail")
    print("ok")


main()
