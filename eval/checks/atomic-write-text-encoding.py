"""Check: `write_file_atomic` names its parameter `text_encoding`, keeps no `encoding`, and every call to it in
the package that passed the old name passes the new one."""

import ast
import pathlib
import sys

MODULE = pathlib.Path("src/serena/util/file_system.py")
CALLERS = [pathlib.Path("src/serena/code_editor.py"), pathlib.Path("src/serena/memories/memory_manager.py")]


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    tree = ast.parse(MODULE.read_text(encoding="utf-8"))
    function = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "write_file_atomic"), None)
    if function is None:
        fail("write_file_atomic not found")
    names = [a.arg for a in function.args.args + function.args.kwonlyargs]
    if "text_encoding" not in names:
        fail("write_file_atomic has no text_encoding parameter")
    if "encoding" in names or function.args.kwarg is not None:
        fail("write_file_atomic still takes encoding or any keyword")
    passed = 0
    for path in CALLERS:
        for node in ast.walk(ast.parse(pathlib.Path(path).read_text(encoding="utf-8"))):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "write_file_atomic"):
                continue
            keywords = [k.arg for k in node.keywords]
            if "encoding" in keywords:
                fail(f"{path} still passes encoding")
            passed += keywords.count("text_encoding")
    if passed < 3:
        fail(f"only {passed} of the 3 calls that passed encoding pass text_encoding")
    print("ok")


main()
