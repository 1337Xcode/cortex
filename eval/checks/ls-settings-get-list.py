"""Check: CustomLSSettings.get_list exists and the launch-command code uses it instead of hand-written list checks."""

import ast
import logging
import pathlib
import sys
import textwrap
from typing import Any

SETTINGS = pathlib.Path("src/solidlsp/settings.py")
PROVIDER = pathlib.Path("src/solidlsp/dependency_provider.py")
KEYS = {"ls_base_cmd", "ls_args", "ls_extra_args"}


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


class _Collector(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.records: list[logging.LogRecord] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


def load_settings_class():
    if not SETTINGS.is_file():
        fail(f"{SETTINGS} not found")
    source = SETTINGS.read_text(encoding="utf-8")
    tree = ast.parse(source)
    segment = None
    for outer in tree.body:
        if isinstance(outer, ast.ClassDef) and outer.name == "SolidLSPSettings":
            for inner in outer.body:
                if isinstance(inner, ast.ClassDef) and inner.name == "CustomLSSettings":
                    segment = textwrap.dedent(ast.get_source_segment(source, inner, padded=True))
    if segment is None:
        fail("SolidLSPSettings.CustomLSSettings not found")
    logger = logging.getLogger("ls_settings_check")
    logger.setLevel(logging.DEBUG)
    logger.propagate = False
    collector = _Collector()
    logger.addHandler(collector)
    namespace = {"Any": Any, "ToStringMixin": object, "log": logger}
    exec(compile(segment, str(SETTINGS), "exec"), namespace)
    return namespace["CustomLSSettings"], collector


def check_settings() -> None:
    cls, collector = load_settings_class()
    if not hasattr(cls, "get_list"):
        fail("CustomLSSettings has no get_list method")

    valid = cls({"cmd": ["a", "b"]})
    if valid.get_list("cmd") != ["a", "b"]:
        fail("get_list does not return a list setting")
    if collector.records:
        fail("get_list warned about a valid list")

    if cls({}).get_list("cmd") is not None or cls(None).get_list("cmd") is not None:
        fail("get_list must return None for a missing setting")
    if collector.records:
        fail("get_list warned about a missing setting")

    bad = cls({"cmd": "not-a-list"})
    if bad.get_list("cmd") is not None:
        fail("get_list must return None for a non-list value")
    warnings = [r for r in collector.records if "cmd" in r.getMessage()]
    if not warnings:
        fail("get_list must log a warning that names the key for a non-list value")

    if cls({"k": 5}).get("k") != 5 or cls({}).get("k", 7) != 7:
        fail("the generic get method changed")


def check_provider() -> None:
    if not PROVIDER.is_file():
        fail(f"{PROVIDER} not found")
    tree = ast.parse(PROVIDER.read_text(encoding="utf-8"))
    used = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        if isinstance(func, ast.Name) and func.id == "isinstance" and len(node.args) == 2:
            if isinstance(node.args[1], ast.Name) and node.args[1].id == "list":
                fail("dependency_provider.py still contains a hand-written isinstance(..., list) check")
        if isinstance(func, ast.Attribute) and node.args and isinstance(node.args[0], ast.Constant):
            key = node.args[0].value
            if func.attr == "get_list" and key in KEYS:
                used.add(key)
            if func.attr == "get" and key in KEYS:
                fail(f"dependency_provider.py still reads {key} with the generic get")
    if used != KEYS:
        fail(f"get_list is not used for {sorted(KEYS - used)}")


def main() -> None:
    check_settings()
    check_provider()
    print("OK: get_list added and used for the three list settings")
    sys.exit(0)


main()
