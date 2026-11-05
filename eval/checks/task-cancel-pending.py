"""Check: TaskExecutor.cancel_pending_tasks cancels queued tasks, leaves the running one, returns the count."""

import logging
import pathlib
import sys
import threading
import time
import types
from contextlib import contextmanager

PATH = pathlib.Path("src/serena/task_executor.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def install_sensai_stubs() -> None:
    """The module only needs a logger factory, a timing context manager and a string mixin from sensai."""

    @contextmanager
    def log_time(*args, **kwargs):
        yield

    sensai = types.ModuleType("sensai")
    util = types.ModuleType("sensai.util")
    logging_mod = types.ModuleType("sensai.util.logging")
    string_mod = types.ModuleType("sensai.util.string")
    logging_mod.getLogger = logging.getLogger
    logging_mod.LogTime = log_time
    string_mod.ToStringMixin = type("ToStringMixin", (), {})
    util.logging = logging_mod
    util.string = string_mod
    sensai.util = util
    sys.modules.update(
        {
            "sensai": sensai,
            "sensai.util": util,
            "sensai.util.logging": logging_mod,
            "sensai.util.string": string_mod,
        }
    )


def wait_until(condition, timeout: float = 5.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return True
        time.sleep(0.02)
    return False


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    install_sensai_stubs()
    namespace: dict = {"__name__": "task_executor_under_check"}
    exec(compile(PATH.read_text(encoding="utf-8"), str(PATH), "exec"), namespace)
    executor_cls = namespace["TaskExecutor"]
    if not hasattr(executor_cls, "cancel_pending_tasks"):
        fail("TaskExecutor has no cancel_pending_tasks method")

    executor = executor_cls("check")
    release = threading.Event()
    ran = []

    running = executor.issue_task(lambda: (release.wait(10), "done")[1], name="running")
    if not wait_until(lambda: len(executor.get_current_tasks()) > 0 and executor.get_current_tasks()[0].is_running):
        fail("the first task never started")
    queued_a = executor.issue_task(lambda: ran.append("a"), name="queued_a")
    queued_b = executor.issue_task(lambda: ran.append("b"), name="queued_b")

    count = executor.cancel_pending_tasks()
    if count != 2:
        release.set()
        fail(f"cancel_pending_tasks returned {count!r}, expected 2")
    if not (queued_a.future.cancelled() and queued_b.future.cancelled()):
        release.set()
        fail("queued tasks were not cancelled")
    if running.future.cancelled():
        release.set()
        fail("the running task was cancelled")

    release.set()
    if running.result(timeout=5) != "done":
        fail("the running task did not finish normally")
    later = executor.issue_task(lambda: 42, name="later")
    if later.result(timeout=5) != 42:
        fail("the executor no longer runs tasks after cancelling")
    time.sleep(0.3)
    if ran:
        fail("a cancelled task still ran")
    if executor.cancel_pending_tasks() != 0:
        fail("cancel_pending_tasks on an idle executor should return 0")

    print("OK: queued tasks are cancelled, the running one is left alone")
    sys.exit(0)


main()
