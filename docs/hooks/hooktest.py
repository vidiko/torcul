# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Shared test helpers for MkDocs hooks under docs/hooks/."""

from __future__ import annotations

import contextlib
import importlib.util
import logging
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import types
    from collections.abc import Iterator
    from pathlib import Path


class ListHandler(logging.Handler):
    """Capture warning records emitted by the hook logger during a test case."""

    def __init__(self) -> None:
        """Start with no captured messages at warning level."""
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        """Store the formatted message of one log record."""
        self.messages.append(record.getMessage())


def load_hook(root: Path, name: str) -> types.ModuleType:
    """Load and execute docs/hooks/<name>.py from root."""
    path = root / "docs/hooks" / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        msg = f"Failed to load docs/hooks/{name}.py"
        raise RuntimeError(msg)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@contextlib.contextmanager
def capture_warnings(logger_name: str) -> Iterator[ListHandler]:
    """Attach a ListHandler to logger_name and detach it on exit."""
    logger = logging.getLogger(logger_name)
    handler = ListHandler()
    logger.addHandler(handler)
    try:
        yield handler
    finally:
        logger.removeHandler(handler)
