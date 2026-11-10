"""Check: SerenaConfig has the heartbeat timeout setting with its default of 5 seconds, the global
configuration template documents it with the same default, and the client's heartbeat request uses
it instead of a fixed number."""

import ast
import pathlib
import re
import sys

CONFIG = pathlib.Path("src/serena/config/serena_config.py")
TEMPLATE = pathlib.Path("src/serena/resources/serena_config.template.yml")
CLIENT = pathlib.Path("src/serena/project_server.py")
KEY = "project_server_heartbeat_timeout"
WANTED = 5.0


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def class_default(tree: ast.AST, cls: str, key: str) -> ast.expr | None:
    """The default the class `cls` gives its field `key`, if it defines the field exactly once."""
    values = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == cls:
            for item in node.body:
                if isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name) and item.target.id == key:
                    values.append(item.value)
    return values[0] if len(values) == 1 else None


def heartbeat_timeouts(tree: ast.AST) -> list[ast.expr]:
    """The `timeout` arguments of every call whose arguments mention the heartbeat endpoint."""
    found = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        if "heartbeat" not in ast.unparse(node):
            continue
        found.extend(keyword.value for keyword in node.keywords if keyword.arg == "timeout")
    return found


def main() -> None:
    if not CONFIG.is_file() or not TEMPLATE.is_file() or not CLIENT.is_file():
        fail("files not found")
    value = class_default(ast.parse(CONFIG.read_text(encoding="utf-8")), "SerenaConfig", KEY)
    if not (isinstance(value, ast.Constant) and isinstance(value.value, (int, float)) and float(value.value) == WANTED):
        fail(f"SerenaConfig.{KEY} is not defined once with the default {WANTED}")
    match = re.search(rf"(?m)^{KEY}:\s*([0-9.]+)\s*(#.*)?$", TEMPLATE.read_text(encoding="utf-8"))
    if match is None:
        fail(f"the template has no {KEY} entry")
    if float(match.group(1)) != WANTED:
        fail(f"the template's default is {match.group(1)}, not {WANTED}")
    timeouts = heartbeat_timeouts(ast.parse(CLIENT.read_text(encoding="utf-8")))
    if not timeouts:
        fail("no heartbeat request with a timeout found in the client")
    if any(isinstance(timeout, ast.Constant) for timeout in timeouts):
        fail("the heartbeat request still uses a fixed timeout")
    if not any(KEY in ast.unparse(timeout) for timeout in timeouts) and KEY not in CLIENT.read_text(encoding="utf-8"):
        fail(f"the client does not read {KEY}")
    print(f"OK: {KEY} is {WANTED} in the class and the template, and the heartbeat uses it")
    sys.exit(0)


main()
