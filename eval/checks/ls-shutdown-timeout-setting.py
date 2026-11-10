"""Check: ProjectConfig has the language-server shutdown timeout setting with its default of 2
seconds, the project template documents it with the same default, and the agent or the project reads
it when the active project is shut down."""

import ast
import pathlib
import re
import sys

CONFIG = pathlib.Path("src/serena/config/serena_config.py")
TEMPLATE = pathlib.Path("src/serena/resources/project.template.yml")
USERS = [pathlib.Path("src/serena/agent.py"), pathlib.Path("src/serena/project.py")]
KEY = "ls_shutdown_timeout"
WANTED = 2.0


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


def reads_key(path: pathlib.Path) -> bool:
    """Whether the file reads an attribute named `KEY`."""
    tree = ast.parse(path.read_text(encoding="utf-8"))
    return any(isinstance(node, ast.Attribute) and node.attr == KEY for node in ast.walk(tree))


def main() -> None:
    if not CONFIG.is_file() or not TEMPLATE.is_file() or not all(path.is_file() for path in USERS):
        fail("files not found")
    value = class_default(ast.parse(CONFIG.read_text(encoding="utf-8")), "ProjectConfig", KEY)
    if not (isinstance(value, ast.Constant) and isinstance(value.value, (int, float)) and float(value.value) == WANTED):
        fail(f"ProjectConfig.{KEY} is not defined once with the default {WANTED}")
    match = re.search(rf"(?m)^{KEY}:\s*([0-9.]+)\s*(#.*)?$", TEMPLATE.read_text(encoding="utf-8"))
    if match is None:
        fail(f"the project template has no {KEY} entry")
    if float(match.group(1)) != WANTED:
        fail(f"the project template's default is {match.group(1)}, not {WANTED}")
    if not any(reads_key(path) for path in USERS):
        fail(f"neither the agent nor the project reads {KEY}")
    print(f"OK: {KEY} is {WANTED} in ProjectConfig and the template, and shutting down reads it")
    sys.exit(0)


main()
