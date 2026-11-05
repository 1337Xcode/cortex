"""Check: execute_shell_command gets an optional timeout_seconds, carried from the tool down to the process."""

import ast
import pathlib
import sys

UTIL = pathlib.Path("src/serena/util/shell.py")
API = pathlib.Path("src/serena/repl/api/shell_api.py")
TOOL = pathlib.Path("src/serena/tools/cmd_tools.py")
PARAM = "timeout_seconds"


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def load(path: pathlib.Path) -> ast.AST:
    if not path.is_file():
        fail(f"{path} not found")
    return ast.parse(path.read_text(encoding="utf-8"))


def find_function(tree: ast.AST, name: str, class_name: str | None = None) -> ast.FunctionDef | None:
    scopes = [tree]
    if class_name is not None:
        scopes = [n for n in ast.walk(tree) if isinstance(n, ast.ClassDef) and n.name == class_name]
    for scope in scopes:
        for node in ast.walk(scope):
            if isinstance(node, ast.FunctionDef) and node.name == name:
                return node
    return None


def get_default(func: ast.FunctionDef, name: str) -> tuple[bool, ast.expr | None]:
    positional = func.args.posonlyargs + func.args.args
    defaults = [None] * (len(positional) - len(func.args.defaults)) + list(func.args.defaults)
    for arg, default in zip(positional, defaults):
        if arg.arg == name:
            return True, default
    for arg, default in zip(func.args.kwonlyargs, func.args.kw_defaults):
        if arg.arg == name:
            return True, default
    return False, None


def names_in(node: ast.AST) -> set[str]:
    return {n.id for n in ast.walk(node) if isinstance(n, ast.Name)}


def passes_param(func: ast.FunctionDef, callee: str) -> bool:
    """Whether the function calls `callee` with an argument that refers to the new parameter."""
    for node in ast.walk(func):
        if not isinstance(node, ast.Call):
            continue
        target = node.func
        target_name = target.attr if isinstance(target, ast.Attribute) else getattr(target, "id", None)
        if target_name != callee:
            continue
        values = list(node.args) + [k.value for k in node.keywords]
        if any(PARAM in names_in(v) for v in values):
            return True
    return False


def main() -> None:
    # util layer: parameter with default None, used as the communicate timeout, process killed on expiry
    util_fn = find_function(load(UTIL), "execute_shell_command")
    if util_fn is None:
        fail("util execute_shell_command not found")
    present, default = get_default(util_fn, PARAM)
    if not present:
        fail("util execute_shell_command has no timeout_seconds parameter")
    if not (isinstance(default, ast.Constant) and default.value is None):
        fail("util timeout_seconds must default to None")
    uses_timeout = any(
        isinstance(n, ast.Call)
        and isinstance(n.func, ast.Attribute)
        and n.func.attr in ("communicate", "wait", "run")
        and any(k.arg == "timeout" and PARAM in names_in(k.value) for k in n.keywords)
        for n in ast.walk(util_fn)
    )
    if not uses_timeout:
        fail("util execute_shell_command does not pass timeout_seconds as timeout= to communicate")
    has_expiry_handling = any(
        (isinstance(n, ast.Attribute) and n.attr == "TimeoutExpired") or (isinstance(n, ast.Name) and n.id == "TimeoutExpired")
        for n in ast.walk(util_fn)
    )

    def callee_name(call: ast.Call) -> str:
        return call.func.attr if isinstance(call.func, ast.Attribute) else getattr(call.func, "id", "")

    kills = any(
        isinstance(n, ast.Call) and ("kill" in callee_name(n) or "terminate" in callee_name(n)) for n in ast.walk(util_fn)
    )
    if not (has_expiry_handling and kills):
        fail("util execute_shell_command does not handle TimeoutExpired by killing the process")

    # API layer
    api_fn = find_function(load(API), "execute_shell_command", "ShellApi")
    if api_fn is None:
        fail("ShellApi.execute_shell_command not found")
    if not get_default(api_fn, PARAM)[0]:
        fail("ShellApi.execute_shell_command has no timeout_seconds parameter")
    if not passes_param(api_fn, "execute_shell_command"):
        fail("ShellApi does not pass timeout_seconds on to the util function")
    if f":param {PARAM}:" not in (ast.get_docstring(api_fn) or ""):
        fail("ShellApi.execute_shell_command docstring lacks :param timeout_seconds:")

    # tool layer
    tool_fn = find_function(load(TOOL), "apply", "ExecuteShellCommandTool")
    if tool_fn is None:
        fail("ExecuteShellCommandTool.apply not found")
    if not get_default(tool_fn, PARAM)[0]:
        fail("ExecuteShellCommandTool.apply has no timeout_seconds parameter")
    if not passes_param(tool_fn, "execute_shell_command"):
        fail("ExecuteShellCommandTool.apply does not pass timeout_seconds on to the API")
    if f":param {PARAM}:" not in (ast.get_docstring(tool_fn) or ""):
        fail("ExecuteShellCommandTool.apply docstring lacks :param timeout_seconds:")

    print("OK: timeout_seconds flows from the tool through the API to the process")
    sys.exit(0)


main()
