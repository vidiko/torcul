# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Pure unit tests for track_fixtures manifest counts and chapter metadata."""

import unittest

from test_interrupt_cleanup import FIXTURES


class FixtureManifestTests(unittest.TestCase):
    """Verify manifest chapter and per-kind stream counts without running FFmpeg."""

    def test_f02_has_three_chapters_and_others_zero(self):
        """Only f02_anime_ja.mkv declares three chapters; every other fixture has zero."""
        rows = {
            fixture["name"]: FIXTURES.manifest_row(fixture)
            for fixture in FIXTURES.FIXTURES
        }
        self.assertEqual(rows["f02_anime_ja.mkv"]["chapters"], 3)
        for name, row in rows.items():
            if name != "f02_anime_ja.mkv":
                self.assertEqual(row["chapters"], 0, name)

    def test_manifest_counts_match_track_tables_and_expect(self):
        """Source and output per-kind counts derive directly from tracks and expect."""
        rows = {
            fixture["name"]: (fixture, FIXTURES.manifest_row(fixture))
            for fixture in FIXTURES.FIXTURES
        }
        self.assertEqual(
            rows["f01_movie_en_de_extra.mkv"][1]["counts"],
            {
                "source": {"video": 2, "audio": 4, "subtitle": 7, "attachment": 0},
                "output": {"video": 2, "audio": 2, "subtitle": 5, "attachment": 0},
            },
        )
        self.assertEqual(
            rows["f02_anime_ja.mkv"][1]["counts"],
            {
                "source": {"video": 1, "audio": 4, "subtitle": 5, "attachment": 2},
                "output": {"video": 1, "audio": 3, "subtitle": 3, "attachment": 2},
            },
        )
        for name, (fixture, row) in rows.items():
            counts = row["counts"]
            self.assertEqual(
                sum(counts["source"].values()), 1 + len(fixture["tracks"]), name
            )
            self.assertEqual(
                sum(counts["output"].values()), len(fixture["expect"]), name
            )

    def test_chapter_metadata_evenly_spaces_chapters(self):
        """FFMETADATA1 generation spaces chapter ranges evenly in milliseconds."""
        text = FIXTURES.chapter_metadata(2, 3)
        self.assertTrue(text.startswith(";FFMETADATA1\n"))
        self.assertIn("START=0\nEND=666\ntitle=Chapter 1\n", text)
        self.assertIn("START=666\nEND=1333\ntitle=Chapter 2\n", text)
        self.assertIn("START=1333\nEND=2000\ntitle=Chapter 3\n", text)


if __name__ == "__main__":
    unittest.main()
