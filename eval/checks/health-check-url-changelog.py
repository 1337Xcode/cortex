"""Check: the error the project server client raises when its health check fails names the server's
URL, and the changelog's unreleased section gained a bullet about the change. At the pinned commit
that section holds 13 bullets, counted by hand."""

import ast
import pathlib
import re
import sys

CLIENT = pathlib.Path("src/serena/project_server.py")
CHANGELOG = pathlib.Path("CHANGELOG.md")
BULLETS_BEFORE = 13
TOPIC = re.compile(r"heartbeat|health check|health-check|project server|projectserver", re.IGNORECASE)


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def failure_messages(tree: ast.AST) -> list[str]:
    """The source of every `ConnectionError` raised in an `except` of `RequestException`."""
    found = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.ExceptHandler) or node.type is None:
            continue
        if "RequestException" not in ast.unparse(node.type):
            continue
        for inner in ast.walk(node):
            if isinstance(inner, ast.Raise) and inner.exc is not None and "ConnectionError" in ast.unparse(inner.exc):
                found.append(ast.unparse(inner.exc))
    return found


def unreleased_bullets(text: str) -> list[str]:
    """The bullet lines of the changelog's first section, up to the next top-level heading."""
    lines = text.splitlines()
    end = next((at for at, line in enumerate(lines[1:], 1) if line.startswith("# ")), len(lines))
    return [line for line in lines[:end] if re.match(r"^\s*- ", line)]


def main() -> None:
    if not CLIENT.is_file() or not CHANGELOG.is_file():
        fail("files not found")
    messages = failure_messages(ast.parse(CLIENT.read_text(encoding="utf-8")))
    if not messages:
        fail("no ConnectionError raised when the health check request fails")
    if not all("_base_url" in message or "url" in message.lower() for message in messages):
        fail("the health check's failure does not name the server's URL")
    bullets = unreleased_bullets(CHANGELOG.read_text(encoding="utf-8"))
    if len(bullets) <= BULLETS_BEFORE:
        fail(f"the unreleased section has {len(bullets)} bullets, none added")
    if not any(TOPIC.search(bullet) for bullet in bullets):
        fail("no unreleased bullet is about the project server's health check")
    print("OK: the failure names the URL, and the changelog says so")
    sys.exit(0)


main()
