"""Check: MultiLangPromptCollection.get_prompt_list must honour the collection's fallback mode."""

import ast
import pathlib
import sys

PATH = pathlib.Path("src/interprompt/multilang_prompt.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def find_method(tree: ast.AST, class_name: str, method_name: str) -> ast.FunctionDef | None:
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == class_name:
            for item in node.body:
                if isinstance(item, ast.FunctionDef) and item.name == method_name:
                    return item
    return None


def mentions_fallback(node: ast.AST) -> bool:
    for sub in ast.walk(node):
        if isinstance(sub, ast.Attribute) and "fallback" in sub.attr:
            return True
        if isinstance(sub, ast.Name) and "fallback" in sub.id:
            return True
    return False


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    tree = ast.parse(PATH.read_text(encoding="utf-8"))
    method = find_method(tree, "MultiLangPromptCollection", "get_prompt_list")
    if method is None:
        fail("MultiLangPromptCollection.get_prompt_list not found")
    calls = [
        n
        for n in ast.walk(method)
        if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "get_item"
    ]
    if not calls:
        fail("get_prompt_list no longer calls get_item")
    for call in calls:
        keyword_ok = any(k.arg == "fallback_mode" and mentions_fallback(k.value) for k in call.keywords)
        positional_ok = len(call.args) >= 2 and mentions_fallback(call.args[1])
        if keyword_ok or positional_ok:
            print("OK: get_prompt_list passes the fallback mode to get_item")
            sys.exit(0)
    fail("get_prompt_list does not pass the collection's fallback mode to get_item")


main()
