"""Check: ToolUsageStats.get_stats must not register a tool that was never recorded."""

import __future__
import ast
import pathlib
import sys
import threading
from collections import defaultdict
from copy import copy
from dataclasses import asdict, dataclass

PATH = pathlib.Path("src/serena/analytics.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


class _StubEstimator:
    """Counts one token per character."""

    def estimate_token_count(self, text: str) -> int:
        return len(text)


class _StubChoice:
    value = "STUB"

    def load_estimator(self) -> _StubEstimator:
        return _StubEstimator()


class _StubRegistered:
    TIKTOKEN_GPT4O = _StubChoice()


def load_class():
    if not PATH.is_file():
        fail(f"{PATH} not found")
    source = PATH.read_text(encoding="utf-8")
    tree = ast.parse(source)
    segment = None
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name == "ToolUsageStats":
            segment = ast.get_source_segment(source, node)
    if segment is None:
        fail("ToolUsageStats not found")
    namespace = {
        "threading": threading,
        "defaultdict": defaultdict,
        "copy": copy,
        "dataclass": dataclass,
        "asdict": asdict,
        "RegisteredTokenCountEstimator": _StubRegistered,
    }
    flags = __future__.annotations.compiler_flag
    exec(compile(segment, str(PATH), "exec", flags=flags, dont_inherit=True), namespace)
    return namespace["ToolUsageStats"]


def main() -> None:
    stats_cls = load_class()
    stats = stats_cls(_StubRegistered.TIKTOKEN_GPT4O)

    unknown = stats.get_stats("never_called")
    if (unknown.num_times_called, unknown.input_tokens, unknown.output_tokens) != (0, 0, 0):
        fail("get_stats of an unknown tool is not an all-zero entry")
    if "never_called" in stats.get_tool_stats_dict():
        fail("get_stats registered the unknown tool in the full statistics")

    stats.record_tool_usage("known", "abcd", "ef")
    entry = stats.get_stats("known")
    if (entry.num_times_called, entry.input_tokens, entry.output_tokens) != (1, 4, 2):
        fail("get_stats of a recorded tool returns wrong numbers")
    entry.num_times_called = 99
    if stats.get_stats("known").num_times_called != 1:
        fail("get_stats no longer returns a copy")
    stats.get_stats("another_unknown")
    if sorted(stats.get_tool_stats_dict()) != ["known"]:
        fail("reading stats changed the set of recorded tools")

    stats.record_tool_usage("never_called", "xyz", "")
    if stats.get_stats("never_called").num_times_called != 1:
        fail("recording a previously queried tool no longer works")

    print("OK: get_stats leaves the recorded tools untouched")
    sys.exit(0)


main()
