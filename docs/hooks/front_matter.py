# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Warn when MkDocs cannot read a page's front matter."""

# MkDocs 1.6 reads a page's leading YAML block in mkdocs.utils.meta.get_data
# and drops any error: the block stays in the page body, the page loses its
# metadata, and mkdocs build --strict still passes. The hook repeats that check
# on the raw source MkDocs read (page.file.content_string) and logs a warning
# naming the page and the reason, so the strict build fails. The Markdown
# passed to the hook is no signal (a valid block may be followed by thematic
# breaks), and neither is page.meta: plugins such as
# git-revision-date-localized add keys to it before hooks run.

from __future__ import annotations

import logging
from typing import Any

import yaml
from mkdocs.utils.meta import YAML_RE

_LOG = logging.getLogger("mkdocs.hooks.front_matter")


def front_matter_error(source: str) -> str | None:
    """Return why a leading front-matter block is not a mapping, or None."""
    match = YAML_RE.match(source)
    if match is None:
        return None
    try:
        data = yaml.safe_load(match.group(1))
    # get_data drops every exception; YAML input raises these two in practice
    # (ValueError: a date such as 2026-13-45).
    except (yaml.YAMLError, ValueError) as exc:
        return " ".join(str(exc).split())
    if isinstance(data, dict):
        return None
    return f"the block is a {type(data).__name__}, not a mapping"


def on_page_markdown(markdown: str, page: Any, config: Any, files: Any) -> str:
    """Warn when MkDocs cannot read the page's front matter."""
    del config, files
    src = getattr(page.file, "src_uri", None) or page.file.src_path
    # The source comes from the file again, so a source replaced by a plugin's
    # on_page_read_source is not seen; mkdocs.yml configures no such plugin.
    try:
        source = page.file.content_string
    except (OSError, ValueError) as exc:
        _LOG.warning("front matter of %s could not be checked: %s", src, exc)
        return markdown
    reason = front_matter_error(source)
    if reason is not None:
        _LOG.warning("front matter of %s was not read: %s", src, reason)
    return markdown
