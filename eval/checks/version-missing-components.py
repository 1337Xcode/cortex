"""Check: Version comparisons treat components missing from the version as zero (no IndexError)."""

import pathlib
import sys

PATH = pathlib.Path("src/serena/util/version.py")


def fail(message: str) -> None:
    print("FAIL: " + message)
    sys.exit(1)


def expect(label: str, func, expected: bool) -> None:
    try:
        actual = func()
    except Exception as e:  # noqa: BLE001
        fail(f"{label} raised {type(e).__name__}")
    if actual is not expected:
        fail(f"{label} returned {actual}, expected {expected}")


def main() -> None:
    if not PATH.is_file():
        fail(f"{PATH} not found")
    namespace: dict = {}
    exec(compile(PATH.read_text(encoding="utf-8"), str(PATH), "exec"), namespace)
    version_cls = namespace["Version"]

    short = version_cls("1.2")
    expect("1.2 at_least (1,2,3)", lambda: short.is_at_least(1, 2, 3), False)
    expect("1.2 at_least (1,2,0)", lambda: short.is_at_least(1, 2, 0), True)
    expect("1.2 at_least (1,1,9,9)", lambda: short.is_at_least(1, 1, 9, 9), True)
    expect("1.2 at_least (1,2,0,1)", lambda: short.is_at_least(1, 2, 0, 1), False)
    expect("1.2 at_most (1,2,3)", lambda: short.is_at_most(1, 2, 3), True)
    expect("1.2 at_most (1,1,5)", lambda: short.is_at_most(1, 1, 5), False)
    expect("1.2 at_most (1,2,0,1)", lambda: short.is_at_most(1, 2, 0, 1), True)
    expect("1.2 equal (1,2,0)", lambda: short.is_equal(1, 2, 0), True)
    expect("1.2 equal (1,2,1)", lambda: short.is_equal(1, 2, 1), False)
    expect("1.2 equal (1,3)", lambda: short.is_equal(1, 3), False)

    # behaviour for versions that have all the components must not change
    full = version_cls("1.2.3rc1")
    expect("1.2.3 at_least (1,2)", lambda: full.is_at_least(1, 2), True)
    expect("1.2.3 at_least (1,3)", lambda: full.is_at_least(1, 3), False)
    expect("1.2.3 at_most (1,2)", lambda: full.is_at_most(1, 2), True)
    expect("1.2.3 at_most (1,2,2)", lambda: full.is_at_most(1, 2, 2), False)
    expect("1.2.3 equal (1,2)", lambda: full.is_equal(1, 2), True)
    expect("1.2.3 equal (1,2,4)", lambda: full.is_equal(1, 2, 4), False)

    print("OK: missing components are treated as zero")
    sys.exit(0)


main()
