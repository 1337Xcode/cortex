"""Check: a project whose folders hold .gitignore files builds its ignore rules from them and from
its configured paths together, so it tells ignored paths from the others; the project and gitignore
tests pass, and the tests were not changed to get there."""

import pathlib
import subprocess
import sys

PROBE = """
import pathlib, sys, tempfile
from serena.config.serena_config import ProjectConfig, SerenaConfig
from serena.project import Project
from solidlsp.ls_config import LanguageServerId

with tempfile.TemporaryDirectory() as root:
    base = pathlib.Path(root).resolve()
    (base / "sub").mkdir()
    for path in ("main.py", "ignored.py", "sub/a.log", "sub/b.py", "build.txt"):
        (base / path).touch()
    (base / ".gitignore").write_text("/ignored.py\\n", encoding="utf-8")
    (base / "sub" / ".gitignore").write_text("*.log\\n", encoding="utf-8")
    project = Project(
        project_root=str(base),
        project_config=ProjectConfig(
            project_name="probe",
            language_servers=[LanguageServerId.PYTHON],
            ignored_paths=["build.txt"],
            ignore_all_files_in_gitignore=True,
        ),
        serena_config=SerenaConfig(gui_log_window=False, web_dashboard=False, ignored_paths=[]),
    )
    expected = {"main.py": False, "sub/b.py": False, "ignored.py": True, "sub/a.log": True, "build.txt": True}
    for path, ignored in expected.items():
        if project.is_ignored_path(path) != ignored:
            sys.exit(f"{path} is {'not ' if ignored else ''}ignored")
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
        fail("a project with .gitignore files does not tell ignored paths from the others")
    tests = subprocess.run(
        [python, "-m", "pytest", "-q", "-p", "no:cacheprovider",
         "test/serena/test_project.py", "test/serena/util/test_file_system.py"],
        capture_output=True, text=True, timeout=240,
    )
    if tests.returncode != 0:
        print(tests.stdout[-2000:])
        fail("the project and gitignore tests fail")
    print("ok")


main()
