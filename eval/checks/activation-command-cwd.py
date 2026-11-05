"""Check: ProjectConfig gets an activation_command_cwd setting that is loaded, documented and used."""

import ast
import pathlib
import re
import sys

CONFIG = pathlib.Path("src/serena/config/serena_config.py")
AGENT = pathlib.Path("src/serena/agent.py")
TEMPLATE = pathlib.Path("src/serena/resources/project.template.yml")
FIELD = "activation_command_cwd"


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def load(path: pathlib.Path) -> ast.AST:
    if not path.is_file():
        fail(f"{path} not found")
    return ast.parse(path.read_text(encoding="utf-8"))


def find_class(tree: ast.AST, name: str) -> ast.ClassDef | None:
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == name:
            return node
    return None


def find_function(scope: ast.AST, name: str) -> ast.FunctionDef | None:
    for node in ast.walk(scope):
        if isinstance(node, ast.FunctionDef) and node.name == name:
            return node
    return None


def main() -> None:
    # dataclass field
    project_config = find_class(load(CONFIG), "ProjectConfig")
    if project_config is None:
        fail("ProjectConfig not found")
    fields = [
        n
        for n in project_config.body
        if isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name) and n.target.id == FIELD
    ]
    if not fields:
        fail("ProjectConfig has no activation_command_cwd field")
    default = fields[0].value
    if not (isinstance(default, ast.Constant) and default.value is None):
        fail("activation_command_cwd must default to None")
    if "str" not in ast.unparse(fields[0].annotation):
        fail("activation_command_cwd must be annotated as a string")

    # loaded from project.yml
    from_dict = find_function(project_config, "_from_dict")
    if from_dict is None:
        fail("ProjectConfig._from_dict not found")
    constructed = any(
        isinstance(n, ast.Call) and any(k.arg == FIELD for k in n.keywords) for n in ast.walk(from_dict)
    )
    if not constructed:
        fail("_from_dict does not pass activation_command_cwd to the constructor")

    # used when running the command
    run_fn = find_function(load(AGENT), "_run_project_activation_command")
    if run_fn is None:
        fail("_run_project_activation_command not found")
    reads_field = any(isinstance(n, ast.Attribute) and n.attr == FIELD for n in ast.walk(run_fn))
    if not reads_field:
        fail("_run_project_activation_command does not read activation_command_cwd")
    popen_cwds = [
        k.value
        for n in ast.walk(run_fn)
        if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "Popen"
        for k in n.keywords
        if k.arg == "cwd"
    ]
    if not popen_cwds:
        fail("the activation command is no longer started with a cwd")
    plain_root = ast.dump(ast.parse("project.project_root", mode="eval").body)
    if all(ast.dump(v) == plain_root for v in popen_cwds):
        fail("the activation command still always runs in the project root")

    # documented in the template
    template = TEMPLATE.read_text(encoding="utf-8") if TEMPLATE.is_file() else ""
    if not re.search(rf"(?m)^{FIELD}:", template):
        fail("project.template.yml has no activation_command_cwd key")

    print("OK: activation_command_cwd is a loaded, documented and used project setting")
    sys.exit(0)


main()
