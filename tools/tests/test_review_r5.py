# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Check displaced fixtures, interrupted measurements and selected tool versions."""

import contextlib
import io
import json
import os
import signal
import tempfile
import time
import types
import unittest
from pathlib import Path
from unittest import mock

from test_interrupt_cleanup import BACKENDS, FIXTURES


def version_result():
    """Stand in for a finished version query with successful output."""
    return types.SimpleNamespace(args=[], returncode=0, stdout=b"version\n", stderr=b"")


class DisplacedFixturesTests(unittest.TestCase):
    """Check the actual displaced set before allowing publication to delete it."""

    def test_foreign_content_added_after_check_is_restored(self):
        """A formerly empty output retains new foreign content and rejects staging."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output, staging = root / "output", root / "staging"
            output.mkdir()
            staging.mkdir()
            (staging / "new").write_bytes(b"new")
            self.assertTrue(FIXTURES.replaceable(output))
            (output / "foreign").write_bytes(b"keep")
            with self.assertRaisesRegex(OSError, str(output)):
                FIXTURES.publish(staging, output)
            self.assertEqual((output / "foreign").read_bytes(), b"keep")
            self.assertEqual((staging / "new").read_bytes(), b"new")
            self.assertEqual(list(root.glob(".output.old-*")), [])

    def test_main_reports_raced_output_without_traceback(self):
        """A foreign set created during generation survives a diagnostic and rc 1."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            self.assertTrue(FIXTURES.replaceable(output))

            def generate(staging, *args):
                """Introduce foreign output after the initial check without FFmpeg."""
                output.mkdir()
                (output / "foreign").write_bytes(b"keep")
                (staging / "new").write_bytes(b"new")

            error = io.StringIO()
            with (
                mock.patch.object(FIXTURES.shutil, "which", return_value="/fixture/ffmpeg"),
                mock.patch.object(FIXTURES, "generate", side_effect=generate),
                contextlib.redirect_stderr(error),
            ):
                result = FIXTURES.main(["track_fixtures", str(output)])
            self.assertEqual(result, 1)
            self.assertIn(str(output), error.getvalue())
            self.assertNotIn("Traceback", error.getvalue())
            self.assertEqual((output / "foreign").read_bytes(), b"keep")
            self.assertEqual(sorted(root.iterdir()), [output])

    def test_owned_output_is_replaced(self):
        """An owned set is replaced and its private holder is removed."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output, staging = root / "output", root / "staging"
            output.mkdir()
            (output / "manifest.json").write_text(json.dumps({"policy": FIXTURES.POLICY}))
            (output / "old").write_bytes(b"old")
            staging.mkdir()
            (staging / "new").write_bytes(b"new")
            FIXTURES.publish(staging, output)
            self.assertEqual((output / "new").read_bytes(), b"new")
            self.assertFalse((output / "old").exists())
            self.assertEqual(sorted(root.iterdir()), [output])

    def test_absent_or_empty_output_is_published(self):
        """First publication and replacement of an empty directory remain valid."""
        for exists in (False, True):
            with self.subTest(exists=exists), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output, staging = root / "output", root / "staging"
                if exists:
                    output.mkdir()
                staging.mkdir()
                (staging / "new").write_bytes(b"new")
                FIXTURES.publish(staging, output)
                self.assertEqual((output / "new").read_bytes(), b"new")
                self.assertEqual(sorted(root.iterdir()), [output])


class MeasurementCleanupTests(unittest.TestCase):
    """Use real short children to check process group cleanup and exception identity."""

    def setUp(self):
        """Give each measurement a private directory and capture its real child."""
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.child = None
        self.popen = BACKENDS.subprocess.Popen
        self.sleep = time.sleep
        self.interrupted = False

    def tearDown(self):
        """Kill and reap any surviving child even when a planted assertion fails."""
        if self.child is not None:
            if self.child.poll() is None:
                pgid = os.getpgid(self.child.pid)
                if pgid > 1 and pgid == self.child.pid and pgid != os.getpgrp():
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(pgid, signal.SIGKILL)
            self.child.wait(timeout=2)

    def spawn(self, *args, **kwargs):
        """Retain the child for process group assertions and failure cleanup."""
        self.child = self.popen(*args, **kwargs)
        return self.child

    def measure(self, args):
        """Run on one allowed CPU while preserving real taskset and child execution."""
        with (
            mock.patch.object(BACKENDS, "CPUS", str(min(os.sched_getaffinity(0)))),
            mock.patch.object(BACKENDS.subprocess, "Popen", side_effect=self.spawn),
        ):
            return BACKENDS.measure(args, Path(self.directory.name) / "stdout")

    def interrupt_once(self, error):
        """Raise once in the poll sleep while allowing bounded cleanup waits."""
        def sleep(duration):
            """Interrupt the first sleep and delegate any cleanup sleeps."""
            if not self.interrupted:
                self.interrupted = True
                raise error
            self.sleep(duration)
        return sleep

    def assert_group_gone(self):
        """Assert that neither the child nor its process group remains."""
        with self.assertRaises(ProcessLookupError):
            os.killpg(self.child.pid, 0)
        self.assertIsNotNone(self.child.returncode)

    def test_interrupt_kills_and_reaps_group(self):
        """A poll sleep interrupt propagates unchanged after the process group is gone."""
        error = KeyboardInterrupt("poll interrupted")
        with (
            mock.patch.object(BACKENDS.time, "sleep", side_effect=self.interrupt_once(error)),
            self.assertRaises(KeyboardInterrupt) as raised,
        ):
            self.measure(["sleep", "2"])
        self.assertIs(raised.exception, error)
        self.assert_group_gone()

    def test_wait_error_kills_and_reaps_group(self):
        """An ordinary wait error receives the same cleanup as an interrupt."""
        error = OSError("wait failed")
        with (
            mock.patch.object(BACKENDS.os, "wait4", side_effect=error),
            self.assertRaises(OSError) as raised,
        ):
            self.measure(["sleep", "2"])
        self.assertIs(raised.exception, error)
        self.assert_group_gone()

    def test_already_reaped_child_preserves_interrupt(self):
        """Cleanup tolerates a child reaped just before the poll sleep is interrupted."""
        error = KeyboardInterrupt("child already exited")

        def interrupt(duration):
            """Reap the real short child before raising the original interrupt."""
            if not self.interrupted:
                self.interrupted = True
                self.child.wait(timeout=2)
                raise error
            self.sleep(duration)

        with (
            mock.patch.object(BACKENDS.os, "wait4", return_value=(0, 0, None)),
            mock.patch.object(BACKENDS.time, "sleep", side_effect=interrupt),
            mock.patch.object(BACKENDS, "stop_group", wraps=BACKENDS.stop_group) as stop,
            self.assertRaises(KeyboardInterrupt) as raised,
        ):
            self.measure(["true"])
        self.assertIs(raised.exception, error)
        stop.assert_called_once_with(self.child)
        self.assert_group_gone()

    def test_cleanup_failure_is_chained_under_interrupt(self):
        """A cleanup error remains visible without replacing the original interrupt."""
        error = KeyboardInterrupt("poll interrupted")
        cleanup_error = OSError("cleanup failed")
        stop = BACKENDS.stop_group

        def fail_after_stop(child):
            """Remove the real child before simulating a secondary cleanup failure."""
            stop(child)
            raise cleanup_error

        with (
            mock.patch.object(BACKENDS.time, "sleep", side_effect=self.interrupt_once(error)),
            mock.patch.object(BACKENDS, "stop_group", side_effect=fail_after_stop),
            self.assertRaises(KeyboardInterrupt) as raised,
        ):
            self.measure(["sleep", "2"])
        self.assertIs(raised.exception, error)
        self.assertIs(raised.exception.__cause__, cleanup_error)
        self.assert_group_gone()

    def test_success_returns_measurement(self):
        """A successful short child returns timing and memory measurements."""
        row = self.measure(["true"])
        self.assertEqual(set(row), {"wall_s", "cpu_s", "max_rss_mib"})
        self.assertTrue(all(value >= 0 for value in row.values()))
        self.assert_group_gone()

    def test_timeout_stops_group_once(self):
        """The existing timeout diagnostic survives exactly one cleanup attempt."""
        with (
            mock.patch.object(BACKENDS, "MAX_POLLS", 0),
            mock.patch.object(BACKENDS, "stop_group", wraps=BACKENDS.stop_group) as stop,
            self.assertRaisesRegex(RuntimeError, "timeout after"),
        ):
            self.measure(["sleep", "2"])
        stop.assert_called_once_with(self.child)
        self.assert_group_gone()


class SelectedVersionsTests(unittest.TestCase):
    """Exercise version collection through case selection without running media tools."""

    def run_selected(self, cases, missing=()):
        """Resolve only selected binaries and stub their version output and case work."""
        allowed = {"ffmpeg", "ffprobe", "gst-discoverer-1.0", "sh", "taskset"} - set(missing)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "result.json"
            lock = BACKENDS.lock_work(root)
            error = io.StringIO()
            try:
                with (
                    mock.patch.object(BACKENDS, "lock_work", return_value=lock),
                    mock.patch.object(BACKENDS.shutil, "which", side_effect=lambda name:
                                      f"/fixture/{name}" if name in allowed else None),
                    mock.patch.object(BACKENDS.subprocess, "run", return_value=
                                      version_result()) as run,
                    mock.patch.object(BACKENDS, "fixture_set", return_value=root / "small"),
                    mock.patch.object(BACKENDS, "probe_cases", return_value=[]),
                    contextlib.redirect_stderr(error),
                ):
                    result = BACKENDS.main(["backends", str(root), str(output), "--cases", cases])
                data = json.loads(output.read_text()) if output.exists() else None
                commands = [call.args[0] for call in run.call_args_list]
                return result, error.getvalue(), data, commands
            finally:
                with contextlib.suppress(OSError):
                    os.close(lock)

    def test_selected_versions_need_no_encoders(self):
        """Probe selection succeeds and records each relevant version exactly once."""
        result, error, data, commands = self.run_selected("probe")
        self.assertEqual((result, error), (0, ""))
        self.assertEqual(data["host"]["ffprobe"], "version")
        self.assertEqual(data["host"]["gst-discoverer-1.0"], "version")
        self.assertCountEqual([command[0] for command in commands],
                              ["/fixture/ffmpeg", "/fixture/ffprobe", "/fixture/gst-discoverer-1.0"])
        self.assertIn(["/fixture/gst-discoverer-1.0", "--gst-version"], commands)

    def test_all_versions_are_retained(self):
        """All selection retains the original version fields and adds the probe tools."""
        with (
            tempfile.TemporaryDirectory(),
            mock.patch.object(BACKENDS.shutil, "which", side_effect=lambda name: f"/fixture/{name}"),
            mock.patch.object(BACKENDS.subprocess, "run", return_value=
                              version_result()) as run,
        ):
            facts = BACKENDS.host_facts()
        versions = {"ffmpeg", "svt", "x265", "dav1d", "gstreamer", "mkvmerge",
                    "ffprobe", "gst-discoverer-1.0"}
        self.assertTrue(versions <= facts.keys())
        self.assertEqual(run.call_count, len(versions))

    def test_missing_selected_tool_is_named(self):
        """A missing required probe tool is reported before any case is run."""
        result, error, data, _ = self.run_selected("probe", missing=("ffprobe",))
        self.assertEqual(result, 1)
        self.assertIn("not on PATH: ffprobe", error)
        self.assertIsNone(data)

    def test_all_still_requires_encoder(self):
        """All cases still fail fast with the name of the first missing encoder."""
        result, error, data, _ = self.run_selected("all")
        self.assertEqual(result, 1)
        self.assertIn("not on PATH: SvtAv1EncApp", error)
        self.assertIsNone(data)
