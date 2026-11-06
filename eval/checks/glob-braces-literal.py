"""Check: `GlobMatcher._expand_braces` returns a malformed brace pattern unchanged as the only
pattern, still expands valid braces, and the module's own test no longer expects malformed braces to
raise."""

import ast
import pathlib
import re
import sys

SOURCE = pathlib.Path("src/serena/util/text_utils.py")
TEST = pathlib.Path("test/serena/test_text_utils.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def expand_braces():
    """The method, taken out of its class and compiled alone: it needs only `re`."""
    tree = ast.parse(SOURCE.read_text(encoding="utf-8"))
    matcher = next((n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "GlobMatcher"), None)
    if matcher is None:
        fail("GlobMatcher not found")
    method = next((n for n in matcher.body if isinstance(n, ast.FunctionDef) and n.name == "_expand_braces"), None)
    if method is None:
        fail("GlobMatcher._expand_braces not found")
    method.decorator_list = []
    module = ast.Module(body=[method], type_ignores=[])
    namespace = {"re": re}
    exec(compile(module, str(SOURCE), "exec"), namespace)  # noqa: S102 - the testbed's own code
    return namespace["_expand_braces"]


def main() -> None:
    expand = expand_braces()
    for pattern in ["src/{}.py", "src/{utils,models", "src/utils,models}.py"]:
        try:
            got = expand(pattern)
        except Exception as error:  # noqa: BLE001 - any error is the old behaviour
            fail(f"{pattern!r} still raises {type(error).__name__}")
        if got != [pattern]:
            fail(f"{pattern!r} gives {got!r}, not itself alone")
    if expand("**/*.{js,ts}") != ["**/*.js", "**/*.ts"]:
        fail("valid braces no longer expand")
    if expand("a/{b,c}/{d,e}.py") != ["a/b/d.py", "a/b/e.py", "a/c/d.py", "a/c/e.py"]:
        fail("two brace sets no longer expand")
    test = TEST.read_text(encoding="utf-8")
    if re.search(r"pytest\.raises\(ValueError,\s*match=\"Invalid glob brace expression\"\)", test):
        fail("the module's test still expects malformed braces to raise")
    print("ok")


main()
