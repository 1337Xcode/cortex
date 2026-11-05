"""Check: the default answer length cap is 100000 in the config class and in the config template."""

import ast
import pathlib
import re
import sys

CONFIG = pathlib.Path("src/serena/config/serena_config.py")
TEMPLATE = pathlib.Path("src/serena/resources/serena_config.template.yml")
KEY = "default_max_tool_answer_chars"
WANTED = 100000


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    if not CONFIG.is_file() or not TEMPLATE.is_file():
        fail("config files not found")
    tree = ast.parse(CONFIG.read_text(encoding="utf-8"))
    values = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == "SerenaConfig":
            for item in node.body:
                if isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name) and item.target.id == KEY:
                    values.append(item.value)
    if len(values) != 1:
        fail("SerenaConfig does not define the field exactly once")
    value = values[0]
    if not (isinstance(value, ast.Constant) and value.value == WANTED):
        fail(f"SerenaConfig.{KEY} default is not {WANTED}")
    match = re.search(rf"(?m)^{KEY}:\s*([0-9_]+)\s*(#.*)?$", TEMPLATE.read_text(encoding="utf-8"))
    if match is None:
        fail("template has no default_max_tool_answer_chars entry")
    if int(match.group(1).replace("_", "")) != WANTED:
        fail(f"template default is {match.group(1)}, not {WANTED}")
    print("OK: both defaults are 100000")
    sys.exit(0)


main()
