"""Check: MemoryManager.memory_exists follows the accessor conventions (sanitise, ignore check, path lookup)."""

import ast
import pathlib
import sys

PATH = pathlib.Path("src/serena/memories/memory_manager.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def self_call_lines(func: ast.FunctionDef, name: str) -> list[int]:
    lines = []
    for node in ast.walk(func):
        if (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == name
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id in ("self", "cls")
        ):
            lines.append(node.lineno)
    return lines


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    tree = ast.parse(PATH.read_text(encoding="utf-8"))
    manager = next((n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "MemoryManager"), None)
    if manager is None:
        fail("MemoryManager not found")
    methods = [n for n in manager.body if isinstance(n, ast.FunctionDef) and n.name == "memory_exists"]
    if len(methods) != 1:
        fail("MemoryManager.memory_exists is missing")
    method = methods[0]
    arg_names = [a.arg for a in method.args.args]
    if arg_names != ["self", "name"]:
        fail(f"memory_exists must take (self, name), got {arg_names}")
    if method.returns is None or ast.unparse(method.returns) != "bool":
        fail("memory_exists must be annotated -> bool")

    sanitize = self_call_lines(method, "_sanitize_name")
    ignored = self_call_lines(method, "_check_not_ignored")
    path_lookup = self_call_lines(method, "get_memory_file_path")
    if not sanitize:
        fail("memory_exists does not sanitise the name")
    if not ignored:
        fail("memory_exists does not refuse ignored memories")
    if not path_lookup:
        fail("memory_exists does not resolve the file with get_memory_file_path")
    if not min(sanitize) < min(ignored):
        fail("memory_exists must sanitise the name before checking the ignore patterns")
    has_exists_call = any(
        isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr in ("exists", "is_file")
        for n in ast.walk(method)
    )
    if not has_exists_call:
        fail("memory_exists does not test the file for existence")
    if not any(isinstance(n, ast.Return) and n.value is not None for n in ast.walk(method)):
        fail("memory_exists returns nothing")
    if not ast.get_docstring(method):
        fail("memory_exists has no docstring")

    print("OK: memory_exists follows the accessor conventions")
    sys.exit(0)


main()
