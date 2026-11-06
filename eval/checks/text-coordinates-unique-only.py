"""Check: `find_text_coordinates` names its parameter `unique_only`, keeps no `require_unique`, and every call to it in
the package that passed the old name passes the new one."""

import ast
import pathlib
import sys

MODULE = pathlib.Path("src/serena/util/text_utils.py")
CALLERS = [pathlib.Path("src/serena/repl/api/jb_api.py"), pathlib.Path("src/serena/repl/api/lsp_api.py")]


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    tree = ast.parse(MODULE.read_text(encoding="utf-8"))
    function = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "find_text_coordinates"), None)
    if function is None:
        fail("find_text_coordinates not found")
    names = [a.arg for a in function.args.args + function.args.kwonlyargs]
    if "unique_only" not in names:
        fail("find_text_coordinates has no unique_only parameter")
    if "require_unique" in names or function.args.kwarg is not None:
        fail("find_text_coordinates still takes require_unique or any keyword")
    passed = 0
    for path in CALLERS:
        for node in ast.walk(ast.parse(pathlib.Path(path).read_text(encoding="utf-8"))):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "find_text_coordinates"):
                continue
            keywords = [k.arg for k in node.keywords]
            if "require_unique" in keywords:
                fail(f"{path} still passes require_unique")
            passed += keywords.count("unique_only")
    if passed < 3:
        fail(f"only {passed} of the 3 calls that passed require_unique pass unique_only")
    print("ok")


main()
