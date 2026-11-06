"""Check: `match_path` names its parameter `root`, keeps no `root_path`, and every call to it in
the package that passed the old name passes the new one."""

import ast
import pathlib
import sys

MODULE = pathlib.Path("src/serena/util/file_system.py")
CALLERS = [pathlib.Path("src/serena/project.py"), pathlib.Path("src/serena/util/file_system.py"), pathlib.Path("src/solidlsp/language_servers/csharp_language_server.py"), pathlib.Path("src/solidlsp/ls.py")]


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    tree = ast.parse(MODULE.read_text(encoding="utf-8"))
    function = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "match_path"), None)
    if function is None:
        fail("match_path not found")
    names = [a.arg for a in function.args.args + function.args.kwonlyargs]
    if "root" not in names:
        fail("match_path has no root parameter")
    if "root_path" in names or function.args.kwarg is not None:
        fail("match_path still takes root_path or any keyword")
    passed = 0
    for path in CALLERS:
        for node in ast.walk(ast.parse(pathlib.Path(path).read_text(encoding="utf-8"))):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "match_path"):
                continue
            keywords = [k.arg for k in node.keywords]
            if "root_path" in keywords:
                fail(f"{path} still passes root_path")
            passed += keywords.count("root")
    if passed < 5:
        fail(f"only {passed} of the 5 calls that passed root_path pass root")
    print("ok")


main()
