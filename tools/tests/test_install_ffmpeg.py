# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Unit tests for pinned CI FFmpeg installation and workflow wiring."""

import hashlib
import io
import os
import re
import tarfile
import tempfile
import types
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

            def fake_open(url, timeout=None):
                self.assertEqual(timeout, installer.NETWORK_TIMEOUT_S)
                return io.BytesIO(b"not-the-expected-ffmpeg-tarball")

            opener = mock.Mock()
            opener.open.side_effect = fake_open
            with mock.patch.object(
                installer.urllib.request, "build_opener", return_value=opener
            ) as mocked_build_opener:
                rc = installer.main(
                    ["install_ffmpeg.py", str(prefix), "--sha256", "0" * 64]
                )
            mocked_build_opener.assert_called_once_with(
                installer.ReleaseRedirectHandler
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
            opener = mock.Mock()
            opener.open.return_value = io.BytesIO(payload)
            with mock.patch.object(
                installer.urllib.request,
                "build_opener",
                return_value=opener,
            ) as mocked_build_opener:
                installer.download_and_verify(
                    installer.FFMPEG_URL, archive, expected
                )
            mocked_build_opener.assert_called_once_with(
                installer.ReleaseRedirectHandler
            )
            opener.open.assert_called_once_with(
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
                            installer.urllib.request, "build_opener"
                        ) as mocked_build_opener,
                        self.assertRaises(ValueError) as raised,
                    ):
                        installer.download_and_verify(
                            url, archive, installer.FFMPEG_SHA256
                        )
                    self.assertIn(url, str(raised.exception))
                    mocked_build_opener.assert_not_called()
                    self.assertFalse(archive.exists())
                    with self.assertRaises(ValueError) as direct:
                        installer.require_release_url(url)
                    self.assertIn(url, str(direct.exception))

    def test_release_redirect_handler_validates_redirect_targets(self):
        """Redirect targets outside https://ffmpeg.org fail before any follow-up request."""
        installer = load_tool("ci/install_ffmpeg.py")
        handler = installer.ReleaseRedirectHandler()
        req = installer.urllib.request.Request("https://ffmpeg.org/releases/x")
        rejected = (
            "http://ffmpeg.org/releases/x",
            "https://evil.example/x",
            "https://ffmpeg.org:8443/x",
            "https://ffmpeg.org:443/x",
            "https://user@ffmpeg.org/releases/x",
            "https://sub.ffmpeg.org/releases/x",
        )
        for bad_url in rejected:
            with (
                self.subTest(url=bad_url),
                self.assertRaises(ValueError) as raised,
            ):
                handler.redirect_request(req, None, 302, "Found", {}, bad_url)
            self.assertIn(bad_url, str(raised.exception))

        allowed = "https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz"
        redirected = handler.redirect_request(req, None, 302, "Found", {}, allowed)
        self.assertIsInstance(redirected, installer.urllib.request.Request)
        self.assertEqual(redirected.full_url, allowed)

        parent = mock.Mock()
        handler.add_parent(parent)
        req.timeout = installer.NETWORK_TIMEOUT_S
        handler.http_error_302(
            req,
            io.BytesIO(),
            302,
            "Found",
            {"location": "/releases/ffmpeg-9.0.2.tar.xz"},
        )
        self.assertEqual(parent.open.call_args.args[0].full_url, allowed)
        parent.reset_mock()
        with self.assertRaises(ValueError):
            handler.http_error_302(
                req, io.BytesIO(), 302, "Found", {"location": "//evil.example/x"}
            )
        parent.open.assert_not_called()


def populate_prefix(installer, prefix, sha=None, *, write_stamp=True):
    """Create the expected bin/ffmpeg, pkgconfig and optional stamp files."""
    (prefix / "bin").mkdir(parents=True, exist_ok=True)
    (prefix / "lib" / "pkgconfig").mkdir(parents=True, exist_ok=True)
    (prefix / "bin" / "ffmpeg").write_bytes(b"#!/bin/sh\n")
    (prefix / "lib" / "pkgconfig" / "libavformat.pc").write_text(
        "Name: libavformat\n", encoding="utf-8"
    )
    if write_stamp:
        digest = sha or installer.FFMPEG_SHA256
        (prefix / installer.STAMP_NAME).write_text(
            f"{installer.expected_stamp(digest)}\n", encoding="utf-8"
        )


def make_small_archive(archive_path, installer):
    """Write a minimal valid `.tar.xz` with two regular files."""
    buf = io.BytesIO()
    base = f"ffmpeg-{installer.FFMPEG_VERSION}"
    with tarfile.open(fileobj=buf, mode="w:xz") as tar:
        for rel, content in (("RELEASE", f"{installer.FFMPEG_VERSION}\n"), ("README", "ok\n")):
            payload = content.encode("utf-8")
            info = tarfile.TarInfo(name=f"{base}/{rel}")
            info.size = len(payload)
            tar.addfile(info, io.BytesIO(payload))
    archive_path.write_bytes(buf.getvalue())


class InstallFfmpegRoundOneTests(unittest.TestCase):
    """Cover C7 probe failures, install lifecycle, symlink args, extract deadline and OS check."""

    def test_verify_installed_handles_probe_errors_empty_and_valid_stdout(self):
        """Probe failures or empty stdout return False; matching version tag returns True."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "prefix"
            populate_prefix(installer, prefix)
            errors = (
                installer.subprocess.CalledProcessError(127, ["ffmpeg", "-version"]),
                installer.subprocess.TimeoutExpired(["ffmpeg", "-version"], 30),
                OSError("exec format error"),
            )
            for err in errors:
                with (
                    self.subTest(err=type(err).__name__),
                    mock.patch.object(installer, "run_checked", side_effect=err),
                ):
                    self.assertFalse(
                        installer.verify_installed(prefix, installer.FFMPEG_SHA256)
                    )
            empty = types.SimpleNamespace(stdout=b"")
            with mock.patch.object(installer, "run_checked", return_value=empty):
                self.assertFalse(
                    installer.verify_installed(prefix, installer.FFMPEG_SHA256)
                )
            good = types.SimpleNamespace(
                stdout=f"ffmpeg version {installer.FFMPEG_TAG} Copyright\n".encode()
            )
            with mock.patch.object(installer, "run_checked", return_value=good):
                self.assertTrue(
                    installer.verify_installed(prefix, installer.FFMPEG_SHA256)
                )

    def test_install_fresh_and_valid_cached_prefix(self):
        """Fresh install writes stamp; valid cached prefix skips download and build."""
        installer = load_tool("ci/install_ffmpeg.py")
        sha = installer.FFMPEG_SHA256
        good = types.SimpleNamespace(
            stdout=f"ffmpeg version {installer.FFMPEG_TAG}\n".encode()
        )
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "prefix"
            with (
                mock.patch.object(installer, "download_and_verify") as dl,
                mock.patch.object(installer, "safe_extract") as ext,
                mock.patch.object(
                    installer,
                    "build_from_source",
                    side_effect=lambda src, dst: populate_prefix(
                        installer, dst, sha, write_stamp=False
                    ),
                ) as bld,
                mock.patch.object(installer, "run_checked", return_value=good),
            ):
                installer.install(prefix, sha)
            dl.assert_called_once()
            ext.assert_called_once()
            bld.assert_called_once()
            self.assertEqual(
                (prefix / installer.STAMP_NAME).read_text(encoding="utf-8").strip(),
                installer.expected_stamp(sha),
            )
            with (
                mock.patch.object(installer, "download_and_verify") as dl2,
                mock.patch.object(installer, "safe_extract") as ext2,
                mock.patch.object(installer, "build_from_source") as bld2,
                mock.patch.object(installer, "run_checked", return_value=good),
            ):
                installer.install(prefix, sha)
            dl2.assert_not_called()
            ext2.assert_not_called()
            bld2.assert_not_called()

    def test_install_rebuilds_cached_prefix_when_probe_fails(self):
        """A cached prefix whose probe raises CalledProcessError is wiped and rebuilt."""
        installer = load_tool("ci/install_ffmpeg.py")
        sha = installer.FFMPEG_SHA256
        good = types.SimpleNamespace(
            stdout=f"ffmpeg version {installer.FFMPEG_TAG}\n".encode()
        )
        probe_err = installer.subprocess.CalledProcessError(1, ["ffmpeg", "-version"])
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "prefix"
            populate_prefix(installer, prefix, sha, write_stamp=True)
            stale_marker = prefix / "stale.txt"
            stale_marker.write_text("old", encoding="utf-8")
            with (
                mock.patch.object(installer, "download_and_verify") as dl,
                mock.patch.object(installer, "safe_extract") as ext,
                mock.patch.object(
                    installer,
                    "build_from_source",
                    side_effect=lambda src, dst: populate_prefix(
                        installer, dst, sha, write_stamp=False
                    ),
                ) as bld,
                mock.patch.object(
                    installer, "run_checked", side_effect=[probe_err, good]
                ),
            ):
                installer.install(prefix, sha)
            dl.assert_called_once()
            ext.assert_called_once()
            bld.assert_called_once()
            self.assertFalse(stale_marker.exists())
            self.assertTrue((prefix / installer.STAMP_NAME).is_file())

    def test_install_cleans_prefix_on_build_or_post_probe_failure(self):
        """Build exceptions and post-install probe failures remove prefix and propagate."""
        installer = load_tool("ci/install_ffmpeg.py")
        sha = installer.FFMPEG_SHA256
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "build-fail"
            boom = RuntimeError("make failed")

            def partial_build(_src, dst):
                populate_prefix(installer, dst, sha, write_stamp=False)
                raise boom

            with (
                mock.patch.object(installer, "download_and_verify"),
                mock.patch.object(installer, "safe_extract"),
                mock.patch.object(
                    installer, "build_from_source", side_effect=partial_build
                ),
                self.assertRaises(RuntimeError) as raised,
            ):
                installer.install(prefix, sha)
            self.assertIs(raised.exception, boom)
            self.assertFalse(os.path.lexists(prefix))

            post_fail = Path(directory) / "probe-fail"
            bad_probe = types.SimpleNamespace(stdout=b"ffmpeg version n8.0\n")
            with (
                mock.patch.object(installer, "download_and_verify"),
                mock.patch.object(installer, "safe_extract"),
                mock.patch.object(
                    installer,
                    "build_from_source",
                    side_effect=lambda src, dst: populate_prefix(
                        installer, dst, sha, write_stamp=False
                    ),
                ),
                mock.patch.object(installer, "run_checked", return_value=bad_probe),
                self.assertRaisesRegex(RuntimeError, "post-install version check"),
            ):
                installer.install(post_fail, sha)
            self.assertFalse(os.path.lexists(post_fail))

    def test_parse_args_keeps_symlink_unresolved(self):
        """parse_args preserves symlink paths so prepare_prefix detects is_symlink()."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real = root / "real_dir"
            real.mkdir()
            sym = root / "sym_prefix"
            sym.symlink_to(real)
            parsed, digest = installer.parse_args(["install_ffmpeg.py", str(sym)])
            self.assertTrue(parsed.is_symlink())
            self.assertEqual(digest, installer.FFMPEG_SHA256)
            parsed2, digest2 = installer.parse_args(
                ["install_ffmpeg.py", str(sym), "--sha256", "a" * 64]
            )
            self.assertTrue(parsed2.is_symlink())
            self.assertEqual(digest2, "a" * 64)

    def test_safe_extract_enforces_deadline_and_extracts_valid_tar(self):
        """safe_extract raises TimeoutError naming the archive on expiry and unpacks valid tar."""
        installer = load_tool("ci/install_ffmpeg.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "sample.tar.xz"
            make_small_archive(archive, installer)
            with mock.patch(
                "time.monotonic",
                side_effect=[10.0, 10.0 + getattr(installer, "EXTRACT_TIMEOUT_S", 300) + 1.0],
            ):
                with self.assertRaises(TimeoutError) as raised:
                    installer.safe_extract(archive, root / "timed_out")
                self.assertIn(str(archive), str(raised.exception))

            out_dir = root / "extracted"
            installer.safe_extract(archive, out_dir)
            base = out_dir / f"ffmpeg-{installer.FFMPEG_VERSION}"
            self.assertEqual(
                (base / "RELEASE").read_text(encoding="utf-8").strip(),
                installer.FFMPEG_VERSION,
            )
            self.assertEqual((base / "README").read_text(encoding="utf-8"), "ok\n")

    def test_non_posix_os_exits_with_message(self):
        """Non-POSIX os.name exits nonzero with an explicit POSIX requirement message."""
        installer = load_tool("ci/install_ffmpeg.py")
        stderr = io.StringIO()
        with (
            mock.patch.object(installer.os, "name", "nt"),
            mock.patch("sys.stderr", stderr),
        ):
            rc = installer.main(["install_ffmpeg.py", "C:/ffmpeg"])
        self.assertNotEqual(rc, 0)
        self.assertIn("POSIX", stderr.getvalue())

    def test_runtime_env_uses_os_pathsep(self):
        """runtime_env joins PATH and PKG_CONFIG_PATH entries with os.pathsep."""
        installer = load_tool("ci/install_ffmpeg.py")
        prefix = Path("/opt/ci-ffmpeg")
        with (
            mock.patch.dict(
                os.environ,
                {"PATH": "/usr/bin", "PKG_CONFIG_PATH": "/usr/lib/pkgconfig"},
                clear=False,
            ),
            mock.patch.object(installer.os, "pathsep", ";"),
        ):
            env = installer.runtime_env(prefix)
        self.assertEqual(env["PATH"], f"{prefix / 'bin'};/usr/bin")
        self.assertEqual(
            env["PKG_CONFIG_PATH"],
            f"{prefix / 'lib' / 'pkgconfig'};/usr/lib/pkgconfig",
        )


if __name__ == "__main__":
    unittest.main()
