"""Check: the `mode` command group has a `show` subcommand that prints a mode's YAML."""

import ast
import pathlib
import sys

PATH = pathlib.Path("src/serena/cli.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def decorator_name(node: ast.expr) -> str:
    target = node.func if isinstance(node, ast.Call) else node
    return ast.unparse(target)


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    tree = ast.parse(PATH.read_text(encoding="utf-8"))
    group = next((n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "ModeCommands"), None)
    if group is None:
        fail("ModeCommands not found")
    shows = [n for n in group.body if isinstance(n, ast.FunctionDef) and n.name == "show"]
    if len(shows) != 1:
        fail("ModeCommands has no show method")
    show = shows[0]

    names = [decorator_name(d) for d in show.decorator_list]
    if "staticmethod" not in names:
        fail("show must be a staticmethod like its siblings")
    commands = [d for d in show.decorator_list if isinstance(d, ast.Call) and decorator_name(d) == "click.command"]
    if not commands or not commands[0].args or ast.literal_eval(commands[0].args[0]) != "show":
        fail('show must be registered with @click.command("show", ...)')
    if not any(k.arg == "help" for k in commands[0].keywords):
        fail("the command needs a help text")
    if not any(k.arg == "context_settings" for k in commands[0].keywords):
        fail("the command should pass context_settings like its siblings")
    arguments = [d for d in show.decorator_list if isinstance(d, ast.Call) and decorator_name(d) == "click.argument"]
    if not arguments or not arguments[0].args or ast.literal_eval(arguments[0].args[0]) != "mode_name":
        fail('show must take a click.argument("mode_name")')
    if [a.arg for a in show.args.args] != ["mode_name"]:
        fail("show must take exactly the parameter mode_name")

    body_nodes = list(ast.walk(show))
    attrs = {n.attr for n in body_nodes if isinstance(n, ast.Attribute)}
    if "get_path" not in attrs:
        fail("show does not locate the mode file with SerenaAgentMode.get_path")
    if "list_registered_mode_names" not in attrs:
        fail("show does not consult the registered mode names for unknown modes")
    if not ({"UsageError", "ClickException", "BadParameter"} & attrs):
        fail("show does not raise a click error for an unknown mode")
    reads_file = "read_text" in attrs or any(
        isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id == "open" for n in body_nodes
    )
    if not reads_file:
        fail("show does not read the mode file")
    if "echo" not in attrs:
        fail("show does not print with click.echo")

    print("OK: mode show is registered and prints the mode file")
    sys.exit(0)


main()
