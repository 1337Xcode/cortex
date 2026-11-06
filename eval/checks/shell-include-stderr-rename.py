"""Check: `execute_shell_command` names its parameter `include_stderr`, keeps no `capture_stderr`,
and every call to it in the package passes the new name."""

import ast
import pathlib
import sys

SHELL = pathlib.Path("src/serena/util/shell.py")
CALLERS = [pathlib.Path("src/serena/config/client_setup.py"), pathlib.Path("src/serena/repl/api/shell_api.py")]


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def main() -> None:
    tree = ast.parse(SHELL.read_text(encoding="utf-8"))
    function = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "execute_shell_command"), None)
    if function is None:
        fail("execute_shell_command not found")
    names = [a.arg for a in function.args.args + function.args.kwonlyargs]
    if "include_stderr" not in names:
        fail("execute_shell_command has no include_stderr parameter")
    if "capture_stderr" in names or function.args.kwarg is not None:
        fail("execute_shell_command still takes capture_stderr or any keyword")
    passed = 0
    for path in CALLERS:
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "execute_shell_command"):
                continue
            keywords = [k.arg for k in node.keywords]
            if "capture_stderr" in keywords:
                fail(f"{path} still passes capture_stderr")
            passed += keywords.count("include_stderr")
    if passed < 6:
        fail(f"only {passed} of the 6 calls that passed the flag pass include_stderr")
    print("ok")


main()
