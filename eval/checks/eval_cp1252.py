"""A pytest plugin for the evaluation's checks: text files opened without an encoding use cp1252, as
they do under a Windows locale, so that a test of the encoding of a written file means the same on
every platform."""

import builtins
import io

_real_open = io.open


def _open_with_locale_encoding(file, mode="r", buffering=-1, encoding=None, *args, **kwargs):
    if encoding is None and "b" not in mode:
        encoding = "cp1252"
    return _real_open(file, mode, buffering, encoding, *args, **kwargs)


def pytest_configure(config):
    io.open = _open_with_locale_encoding
    builtins.open = _open_with_locale_encoding
