# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Unit tests for pinned CI FFmpeg installation and workflow wiring."""

import hashlib
import io
import os
import re
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from test_interrupt_cleanup import load_tool

REPO_ROOT = Path(__file__).resolve().parents[2]


class InstallFfmpegTests(unittest.TestCase):
    """Check CI workflow pinning and installer integrity guards without compiling."""

    def test_ci_workflow_uses_pinned_ffmpeg_installer(self):
        """CI runs tools/ci/install_ffmpeg.py with SHA-pinned actions/cache."""
        workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("tools/ci/install_ffmpeg.py", workflow)
        self.assertRegex(workflow, r"actions/cache@[0-9a-f]{40}\b")
        self.assertNotIn("libavformat-dev", workflow)
        self.assertNotRegex(workflow, r"apt-get install[^\n]*\bffmpeg\b")

    def test_wrong_sha256_fails_and_leaves_no_prefix(self):
        """A digest mismatch exits nonzero and leaves neither prefix nor staging."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = root / "ci-ffmpeg"

            def fake_urlopen(url, timeout=None):
                self.assertEqual(timeout, installer.NETWORK_TIMEOUT_S)
                return io.BytesIO(b"not-the-expected-ffmpeg-tarball")

            with mock.patch.object(
                installer.urllib.request, "urlopen", side_effect=fake_urlopen
            ):
                rc = installer.main(
                    ["install_ffmpeg.py", str(prefix), "--sha256", "0" * 64]
                )
            self.assertNotEqual(rc, 0)
            self.assertFalse(prefix.exists())
            self.assertEqual(list(root.iterdir()), [])

    def test_unsafe_tar_member_is_rejected(self):
        """Path traversal inside an archive fails extraction before touching prefix."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "bad.tar.xz"
            buf = io.BytesIO()
            with tarfile.open(fileobj=buf, mode="w:xz") as tar:
                info = tarfile.TarInfo(name="../escape.txt")
                payload = b"bad"
                info.size = len(payload)
                tar.addfile(info, io.BytesIO(payload))
            archive.write_bytes(buf.getvalue())
            with self.assertRaises(ValueError):
                installer.safe_extract(archive, root / "extract")

    def test_github_env_receives_prefix_paths(self):
        """GITHUB_ENV receives PATH, PKG_CONFIG_PATH and LD_LIBRARY_PATH entries."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = root / "prefix"
            env_file = root / "github.env"
            with mock.patch.dict(
                os.environ,
                {"PATH": "/usr/bin", "PKG_CONFIG_PATH": "", "LD_LIBRARY_PATH": ""},
                clear=False,
            ):
                installer.write_github_env(prefix, env_file)
            written = env_file.read_text(encoding="utf-8")
            self.assertTrue(
                re.search(
                    rf"^PATH={re.escape(str(prefix / 'bin'))}:", written, re.MULTILINE
                )
            )
            self.assertTrue(
                re.search(
                    rf"^PKG_CONFIG_PATH={re.escape(str(prefix / 'lib' / 'pkgconfig'))}",
                    written,
                    re.MULTILINE,
                )
            )
            self.assertTrue(
                re.search(
                    rf"^LD_LIBRARY_PATH={re.escape(str(prefix / 'lib'))}",
                    written,
                    re.MULTILINE,
                )
            )

    def test_release_url_allowlist_accepts_pinned_ffmpeg_url(self):
        """FFMPEG_URL passes require_release_url and download_and_verify."""
        installer = load_tool("ci/install_ffmpeg.py")
        self.assertIsNone(installer.require_release_url(installer.FFMPEG_URL))
        payload = b"pinned-ffmpeg-archive-bytes"
        expected = hashlib.sha256(payload).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "ffmpeg.tar.xz"
            with mock.patch.object(
                installer.urllib.request,
                "urlopen",
                return_value=io.BytesIO(payload),
            ) as mocked_urlopen:
                installer.download_and_verify(
                    installer.FFMPEG_URL, archive, expected
                )
            mocked_urlopen.assert_called_once_with(
                installer.FFMPEG_URL, timeout=installer.NETWORK_TIMEOUT_S
            )
            self.assertEqual(archive.read_bytes(), payload)

    def test_release_url_allowlist_rejects_disallowed_urls_before_urlopen(self):
        """Disallowed schemes, hosts, credentials and ports fail before urlopen."""
        installer = load_tool("ci/install_ffmpeg.py")
        rejected = (
            "http://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "file:///etc/passwd",
            "ftp://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "https://ffmpeg.org.evil.example/releases/ffmpeg-9.0.2.tar.xz",
            "https://sub.ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "https://user@ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "https://:secret@ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "https://@ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
            "https://ffmpeg.org:8443/releases/ffmpeg-9.0.2.tar.xz",
            "https://ffmpeg.org:443/releases/ffmpeg-9.0.2.tar.xz",
            "https://ffmpeg.org:/releases/ffmpeg-9.0.2.tar.xz",
            "https:///releases/ffmpeg-9.0.2.tar.xz",
            "",
        )
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "never-written.tar.xz"
            for url in rejected:
                with self.subTest(url=url):
                    with (
                        mock.patch.object(
                            installer.urllib.request, "urlopen"
                        ) as mocked_urlopen,
                        self.assertRaises(ValueError) as raised,
                    ):
                        installer.download_and_verify(
                            url, archive, installer.FFMPEG_SHA256
                        )
                    self.assertIn(url, str(raised.exception))
                    mocked_urlopen.assert_not_called()
                    self.assertFalse(archive.exists())
                    with self.assertRaises(ValueError) as direct:
                        installer.require_release_url(url)
                    self.assertIn(url, str(direct.exception))


if __name__ == "__main__":
    unittest.main()
