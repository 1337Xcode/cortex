"""Check: Project.retrieve_content_around_line takes relative_path and all callers use the new name."""

import ast
import pathlib
import sys

PROJECT = pathlib.Path("src/serena/project.py")
CALLERS = [pathlib.Path("src/serena/repl/api/jb_api.py"), pathlib.Path("src/serena/repl/api/lsp_api.py")]
METHOD = "retrieve_content_around_line"
OLD = "relative_file_path"
NEW = "relative_path"


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def load(path: pathlib.Path) -> ast.AST:
    if not path.is_file():
        fail(f"{path} not found")
    return ast.parse(path.read_text(encoding="utf-8"))


def main() -> None:
    definition = next(
        (n for n in ast.walk(load(PROJECT)) if isinstance(n, ast.FunctionDef) and n.name == METHOD),
        None,
    )
    if definition is None:
        fail(f"{METHOD} not found in project.py")
    params = [a.arg for a in definition.args.posonlyargs + definition.args.args + definition.args.kwonlyargs]
    if NEW not in params or OLD in params:
        fail(f"{METHOD} parameters are {params}")
    if params[:2] != ["self", NEW]:
        fail("the path must stay the first parameter after self")
    if any(isinstance(n, ast.Name) and n.id == OLD for n in ast.walk(definition)):
        fail(f"{METHOD} body still refers to {OLD}")

    for path in CALLERS:
        calls = [
            n
            for n in ast.walk(load(path))
            if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == METHOD
        ]
        if not calls:
            fail(f"no call of {METHOD} found in {path.name}")
        for call in calls:
            keywords = [k.arg for k in call.keywords]
            if OLD in keywords:
                fail(f"{path.name} still passes {OLD}=")
            if NEW not in keywords and not call.args:
                fail(f"{path.name} passes no path to {METHOD}")

    print("OK: parameter renamed and callers updated")
    sys.exit(0)


main()
