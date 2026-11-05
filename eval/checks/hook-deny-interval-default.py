"""Check: the reminder hook waits five minutes (300 s) after a deny, and its documentation says so."""

import ast
import pathlib
import sys

PATH = pathlib.Path("src/serena/hooks.py")
CONSTANT = "_MIN_DENY_INTERVAL_SECONDS"


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    tree = ast.parse(PATH.read_text(encoding="utf-8"))
    hook = next(
        (n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "PreToolUseRemindAboutSymbolicToolsHook"),
        None,
    )
    if hook is None:
        fail("PreToolUseRemindAboutSymbolicToolsHook not found")
    counter = next((n for n in hook.body if isinstance(n, ast.ClassDef) and n.name == "ToolUseCounter"), None)
    if counter is None:
        fail("ToolUseCounter not found")
    values = [
        n.value
        for n in counter.body
        if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == CONSTANT for t in n.targets)
    ]
    if len(values) != 1:
        fail(f"{CONSTANT} is not defined exactly once")
    value = values[0]
    if not (isinstance(value, ast.Constant) and value.value == 300):
        fail(f"{CONSTANT} is not 300")
    docs = (ast.get_docstring(hook) or "").lower()
    if "two minutes" in docs or "2 minutes" in docs:
        fail("the class documentation still quotes the old two minute interval")
    if not any(text in docs for text in ("five minutes", "5 minutes", "300 seconds", "300s", "300 s")):
        fail("the class documentation does not state the new interval")
    print("OK: 300 s deny interval, documented")
    sys.exit(0)


main()
