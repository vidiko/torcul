# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Rewrite relative links leaving docs/ to repository URLs."""

# In on_config, the hook reads the repository's tracked files with git ls-files -z
# and registers a Python-Markdown treeprocessor at priority 15 (after inline link
# parsing at priority 20, before MkDocs' relative-path validator at priority 0).
# In on_page_markdown, the hook records the active page URI on the extension. The
# treeprocessor handles relative anchor hrefs that leave docs/ (including ``..``):
# links that step back into docs/ are rewritten to page-relative paths so MkDocs
# validates the target and any heading anchor; links to the repository root
# (``repo_path == "."``) rewrite to the forge tree base URL; links to tracked
# files or directories outside docs/ rewrite to forge blob or tree URLs; and links
# that escape above the repository root or target an untracked repository path log
# a warning so ``mkdocs build --strict`` fails.

from __future__ import annotations

import logging
import os
import posixpath
import shutil
import subprocess  # nosec B404  # Fixed git argv, no shell.
import urllib.parse
from collections.abc import Iterable, Mapping
from pathlib import Path, PurePosixPath
from typing import TYPE_CHECKING, Any

from markdown import Markdown
from markdown.extensions import Extension
from markdown.treeprocessors import Treeprocessor
from mkdocs.exceptions import PluginError

if TYPE_CHECKING:
    # Type only: Python-Markdown builds the element tree; the hook parses no XML.
    # nosemgrep: use-defused-xml  # Type annotation only.
    from xml.etree.ElementTree import Element  # nosec B405  # Type annotation only.

_LOG = logging.getLogger("mkdocs.hooks.repo_links")


def _forge_ref(config: Mapping[str, Any]) -> tuple[str, str]:
    """Return (<repo_url>, <branch>) from config, defaulting branch to HEAD."""
    repo_url = str(config.get("repo_url") or "").rstrip("/")
    edit_uri = str(config.get("edit_uri") or "").strip("/")
    parts = edit_uri.split("/")
    branch = parts[1] if len(parts) > 1 and parts[0] in ("edit", "blob") else "HEAD"
    return repo_url, branch


def _tracked_files(repo_root: Path) -> frozenset[str]:
    """Return the repository's tracked file paths from git ls-files -z."""
    git = shutil.which("git")
    if git is None:
        raise PluginError("repo_links: git is not on PATH")
    try:
        # nosemgrep: dangerous-subprocess-use-audit  # Fixed git argv.
        proc = subprocess.run(  # nosec B603  # Resolved git, fixed argv, no shell or input.
            [git, "ls-files", "-z"],
            cwd=repo_root,
            capture_output=True,
            timeout=30,
            check=True,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        stderr = getattr(exc, "stderr", b"") or b""
        cause = stderr.decode("utf-8", errors="replace").strip() or str(exc)
        raise PluginError(
            f"repo_links: git ls-files failed in {repo_root}: {cause}"
        ) from exc
    output = proc.stdout.decode("utf-8", errors="surrogateescape")
    return frozenset(entry for entry in output.split("\0") if entry)


class _RepoLinksTreeprocessor(Treeprocessor):
    """Treeprocessor that rewrites relative anchor hrefs leaving docs/."""

    def __init__(self, md: Markdown, ext: RepoLinksExtension) -> None:
        """Bind the treeprocessor to its Markdown instance and extension."""
        super().__init__(md)
        self._ext = ext

    def run(self, root: Element) -> Element:
        """Rewrite every anchor href in the parsed element tree."""
        for anchor in root.iter("a"):
            href = anchor.get("href")
            if href is not None:
                anchor.set("href", self._ext.rewrite_href(href))
        return root


class RepoLinksExtension(Extension):
    """Python-Markdown extension that registers the repository link rewriter."""

    def __init__(
        self,
        config: Mapping[str, Any],
        tracked: Iterable[str],
        docs_rel: str = "docs",
        src_uri: str = "index.md",
    ) -> None:
        """Derive the forge URLs from config and index the tracked paths."""
        super().__init__()
        repo_url, branch = _forge_ref(config)
        self._blob_base = f"{repo_url}/blob/{branch}"
        self._tree_base = f"{repo_url}/tree/{branch}"
        self._tracked_files = frozenset(tracked)
        self._tracked_dirs = frozenset(
            parent.as_posix()
            for item in self._tracked_files
            for parent in PurePosixPath(item).parents
            if parent.as_posix() != "."
        )
        self.docs_rel = docs_rel
        self.src_uri = src_uri

    def set_page(self, src_uri: str) -> None:
        """Record the docs-relative URI of the page about to be rendered."""
        self.src_uri = src_uri.replace("\\", "/")

    def _docs_relative_href(
        self,
        repo_path: str,
        page_dir: str,
        parts: urllib.parse.SplitResult,
        suffix: str,
    ) -> str:
        """Return a page-relative href for a link that re-enters docs/."""
        in_docs = posixpath.relpath(repo_path, self.docs_rel)
        rel_page = posixpath.relpath(in_docs, page_dir or ".")
        quoted = urllib.parse.quote(rel_page)
        if parts.path.endswith("/") and not quoted.endswith("/"):
            quoted = f"{quoted}/"
        return f"{quoted}{suffix}"

    def _forge_href(self, href: str, repo_path: str, suffix: str) -> str:
        """Return a forge blob or tree URL for repo_path, or warn if untracked."""
        if repo_path == ".":
            return f"{self._tree_base}{suffix}"
        if repo_path in self._tracked_files:
            base = self._blob_base
        elif repo_path in self._tracked_dirs:
            base = self._tree_base
        else:
            _LOG.warning(
                "Doc file '%s' contains a link '%s' outside docs/, "
                "but '%s' is not a tracked repository path.",
                self.src_uri,
                href,
                repo_path,
            )
            return href
        quoted = urllib.parse.quote(repo_path)
        return f"{base}/{quoted}{suffix}"

    def rewrite_href(self, href: str) -> str:
        """Rewrite one anchor href when it leaves docs/."""
        parts = urllib.parse.urlsplit(href)
        if (
            parts.scheme
            or parts.netloc
            or not parts.path
            or href.startswith(("/", "\\"))
            or parts.path.startswith(("/", "\\"))
        ):
            return href
        page_dir = posixpath.dirname(self.src_uri)
        unquoted = urllib.parse.unquote(parts.path)
        rel_docs = posixpath.normpath(posixpath.join(page_dir, unquoted))
        if rel_docs != ".." and not rel_docs.startswith("../"):
            return href
        repo_path = posixpath.normpath(posixpath.join(self.docs_rel, rel_docs))
        if repo_path == ".." or repo_path.startswith("../"):
            _LOG.warning(
                "Doc file '%s' contains a link '%s' outside docs/, "
                "and '%s' escapes the repository root.",
                self.src_uri,
                href,
                repo_path,
            )
            return href
        query = f"?{parts.query}" if parts.query else ""
        fragment = f"#{parts.fragment}" if parts.fragment else ""
        suffix = f"{query}{fragment}"
        if repo_path == self.docs_rel or repo_path.startswith(f"{self.docs_rel}/"):
            return self._docs_relative_href(repo_path, page_dir, parts, suffix)
        return self._forge_href(href, repo_path, suffix)

    def extendMarkdown(self, md: Markdown) -> None:
        """Register the link-rewriting treeprocessor at priority 15."""
        md.treeprocessors.register(_RepoLinksTreeprocessor(md, self), "repo_links", 15)


def on_config(config: Any) -> Any:
    """Compute tracked repository paths and append RepoLinksExtension."""
    cfg_path = getattr(config, "config_file_path", None) or config["config_file_path"]
    config_file = Path(os.path.abspath(str(cfg_path)))
    repo_root = config_file.parent
    docs_path = getattr(config, "docs_dir", None) or config["docs_dir"]
    docs_dir = Path(os.path.abspath(str(docs_path)))
    try:
        docs_rel = docs_dir.relative_to(repo_root).as_posix()
    except ValueError as exc:
        raise PluginError(
            f"repo_links: docs_dir {docs_dir} is outside {repo_root}"
        ) from exc
    tracked = _tracked_files(repo_root)
    ext = RepoLinksExtension(config, tracked, docs_rel=docs_rel)
    extensions = getattr(config, "markdown_extensions", None)
    if extensions is None:
        extensions = config["markdown_extensions"]
    extensions[:] = [
        item for item in extensions if not isinstance(item, RepoLinksExtension)
    ]
    extensions.append(ext)
    return config


def on_page_markdown(markdown: str, page: Any, config: Any, files: Any) -> str:
    """Tell RepoLinksExtension the current page before rendering."""
    del files
    src_uri = str(getattr(page.file, "src_uri", None) or page.file.src_path)
    extensions = getattr(config, "markdown_extensions", None)
    if extensions is None:
        extensions = config.get("markdown_extensions", ())
    for item in extensions:
        if isinstance(item, RepoLinksExtension):
            item.set_page(src_uri)
    return markdown
