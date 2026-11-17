# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Generate the synthetic track-policy fixture set and its expected plans.

Every fixture is generated from FFmpeg's lavfi test sources, so the set has no
third-party media and no licence of its own. The manifest written beside the
files is the oracle for the track policy of praetor-decided-media-track-policy:
audio keeps English and German, plus the original language when neither is
present or the title is anime; subtitles keep English and German in all
variants; video and attachments are always kept; an undetermined language is
kept and flagged for the check stage.

Usage:
    python3 -I tools/fixtures/track_fixtures.py <out-dir> [--large]

--large writes the bench variant (1920x1080, 120 s) of the first fixture only.
Requires the ffmpeg binary on PATH. Exit 0 on success, 2 on a usage error,
1 when an ffmpeg call fails or times out.
"""

import json
import pathlib
import shutil
import subprocess
import sys

FFMPEG_TIMEOUT_S = 600  # HISS-02: deadline for one ffmpeg call
SMALL = {"size": "320x180", "seconds": 2}
LARGE = {"size": "1920x1080", "seconds": 120}

# One row per track after the video stream: (kind, language, title, dispositions).
# kind: a = audio, s = subtitle, t = attachment (font), c = cover image attachment.
# expect: indices of the output-relevant input streams the policy keeps, in
# stream order (stream 0 is the video); warnings: warning codes the plan raises.
FIXTURES = [
    {
        "name": "f01_movie_en_de_extra.mkv", "original": "en", "anime": False,
        "tracks": [
            ("a", "eng", "English", "default"), ("a", "ger", "Deutsch", ""),
            ("a", "fre", "Francais", ""), ("a", "spa", "Espanol", ""),
            ("s", "eng", "English", ""), ("s", "eng", "English forced", "forced"),
            ("s", "eng", "English SDH", "hearing_impaired"), ("s", "ger", "Deutsch", ""),
            ("s", "ger", "Deutsch forced", "forced"), ("s", "fre", "Francais", ""),
            ("s", "ita", "Italiano", ""), ("c", "", "cover.jpg", ""),
        ],
        "expect": [0, 1, 2, 5, 6, 7, 8, 9, 12], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f02_anime_ja.mkv", "original": "ja", "anime": True,
        "tracks": [
            ("a", "jpn", "Japanese", "default"), ("a", "eng", "English", ""),
            ("a", "ger", "Deutsch", ""), ("a", "fre", "Francais", ""),
            ("s", "eng", "English", "ass"), ("s", "ger", "Deutsch", "ass"),
            ("s", "ger", "Signs and Songs", "ass,forced"), ("s", "spa", "Espanol", "ass"),
            ("s", "fre", "Francais", "ass"), ("t", "", "font-a.ttf", ""),
            ("t", "", "font-b.ttf", ""),
        ],
        "expect": [0, 1, 2, 3, 5, 6, 7, 10, 11], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f03_foreign_only_fr.mkv", "original": "fr", "anime": False,
        "tracks": [
            ("a", "fre", "Francais", "default"), ("a", "ita", "Italiano", ""),
            ("s", "eng", "English", ""), ("s", "fre", "Francais", ""),
        ],
        "expect": [0, 1, 3], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f04_foreign_original_unknown.mkv", "original": None, "anime": False,
        "tracks": [
            ("a", "fre", "Francais", "default"), ("a", "ita", "Italiano", ""),
            ("s", "ger", "Deutsch", ""), ("s", "fre", "Francais", ""),
        ],
        "expect": [0, 1, 2, 3], "warnings": ["original-language-unknown"], "needs_remux": True,
    },
    {
        "name": "f05_undetermined.mkv", "original": "en", "anime": False,
        "tracks": [
            ("a", "und", "", "default"), ("a", "ger", "Deutsch", ""), ("a", "spa", "Espanol", ""),
            ("s", "und", "", ""), ("s", "eng", "English", ""),
        ],
        "expect": [0, 1, 2, 4, 5], "warnings": ["language-undetermined"], "needs_remux": True,
    },
    {
        "name": "f06_already_clean.mkv", "original": "de", "anime": False,
        "tracks": [
            ("a", "deu", "Deutsch", "default"), ("a", "eng", "English", ""),
            ("s", "ger", "Deutsch forced", "forced"), ("s", "eng", "English", ""),
        ],
        "expect": [0, 1, 2, 3, 4], "warnings": [], "needs_remux": False,
    },
    {
        "name": "f07_default_dropped.mkv", "original": "en", "anime": False,
        "tracks": [
            ("a", "fre", "Francais", "default"), ("a", "eng", "English", ""),
            ("s", "eng", "English", ""),
        ],
        "expect": [0, 2, 3], "warnings": ["default-audio-moved"], "needs_remux": True,
    },
    {
        "name": "f08_mp4_en_fr.mp4", "original": "en", "anime": False,
        "tracks": [
            ("a", "eng", "English", "default"), ("a", "fre", "Francais", ""),
            ("s", "eng", "English", "mov_text"), ("s", "fre", "Francais", "mov_text"),
        ],
        "expect": [0, 1, 3], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f09_anime_original_only.mkv", "original": "ja", "anime": True,
        "tracks": [
            ("a", "jpn", "Japanese", "default"), ("s", "eng", "English", "ass"),
            ("s", "ger", "Deutsch", "ass"), ("s", "chi", "Chinese", "ass"),
        ],
        "expect": [0, 1, 2, 3], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f10_commentary.mkv", "original": "en", "anime": False,
        "tracks": [
            ("a", "eng", "English", "default"), ("a", "eng", "Director commentary", "comment"),
            ("a", "ger", "Deutsch", ""), ("a", "jpn", "Japanese", ""),
            ("s", "eng", "Commentary", "comment"),
        ],
        "expect": [0, 1, 2, 3, 5], "warnings": [], "needs_remux": True,
    },
    {
        "name": "f11_no_audio_matches_original.mkv", "original": "ko", "anime": False,
        "tracks": [
            ("a", "fre", "Francais", "default"), ("a", "ita", "Italiano", ""),
            ("s", "eng", "English", ""),
        ],
        "expect": [0, 1, 2, 3], "warnings": ["no-audio-matched"], "needs_remux": False,
    },
]


def run(args):
    """Run one ffmpeg argument list under the deadline; raise on failure."""
    subprocess.run(args, check=True, timeout=FFMPEG_TIMEOUT_S,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def side_files(work):
    """Write the subtitle text, the dummy fonts and the cover image; return their paths."""
    srt = work / "cue.srt"
    srt.write_text("1\n00:00:00,200 --> 00:00:01,500\nfixture cue\n", encoding="utf-8")
    fonts = []
    for name in ("font-a.ttf", "font-b.ttf"):
        font = work / name
        font.write_bytes(b"torcul fixture font placeholder " + name.encode())
        fonts.append(font)
    cover = work / "cover.jpg"
    run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-f", "lavfi", "-i",
         "testsrc2=size=160x90:rate=1", "-frames:v", "1", str(cover)])
    return srt, fonts, cover


def input_args(fixture, geometry, srt):
    """Input arguments: one video source, one sine source per audio track, one SRT per subtitle."""
    secs = str(geometry["seconds"])
    args = ["-f", "lavfi", "-i", f"testsrc2=size={geometry['size']}:rate=24:duration={secs}"]
    audio = [t for t in fixture["tracks"] if t[0] == "a"]
    for number, _ in enumerate(audio):
        args += ["-f", "lavfi", "-i", f"sine=frequency={300 + 110 * number}:duration={secs}"]
    for track in fixture["tracks"]:
        if track[0] == "s":
            args += ["-i", str(srt)]
    return args


def stream_args(fixture, fonts, cover):
    """Map, codec, metadata and disposition arguments for every track of a fixture."""
    args = ["-map", "0:v"]
    counters = {"a": 0, "s": 0, "t": 0}
    source = 1
    font_queue = list(fonts)
    for track in fixture["tracks"]:
        kind, lang, title, flags = track
        if kind in ("t", "c"):
            path = cover if kind == "c" else font_queue.pop(0)
            mime = "image/jpeg" if kind == "c" else "application/x-truetype-font"
            tag = f"-metadata:s:t:{counters['t']}"
            counters["t"] += 1
            args += ["-attach", str(path), tag, f"mimetype={mime}", tag, f"filename={title}"]
            continue
        pos = counters[kind]
        counters[kind] += 1
        args += ["-map", f"{source}:{kind}"]
        source += 1
        spec = f"{kind}:{pos}"
        if lang:
            args += [f"-metadata:s:{spec}", f"language={lang}"]
        if title:
            args += [f"-metadata:s:{spec}", f"title={title}"]
        dispositions = [f for f in flags.split(",") if f in ("default", "forced", "hearing_impaired", "comment")]
        args += [f"-disposition:{spec}", "+".join(dispositions) if dispositions else "0"]
    return args


def codec_args(fixture):
    """Codec arguments: H.264 video, AAC audio, subtitle codec per container and flag."""
    args = ["-c:v", "libx264", "-preset", "ultrafast", "-crf", "35", "-c:a", "aac", "-b:a", "48k"]
    pos = 0
    for track in fixture["tracks"]:
        if track[0] != "s":
            continue
        flags = track[3].split(",")
        codec = "mov_text" if "mov_text" in flags else ("ass" if "ass" in flags else "srt")
        args += [f"-c:s:{pos}", codec]
        pos += 1
    return args


def build(fixture, out_dir, geometry, side):
    """Generate one fixture file."""
    srt, fonts, cover = side
    args = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error"]
    args += input_args(fixture, geometry, srt)
    args += stream_args(fixture, fonts, cover)
    args += codec_args(fixture)
    # -t, not -shortest: the subtitle inputs end at 1.5 s and would cut the file.
    args += ["-t", str(geometry["seconds"]), str(out_dir / fixture["name"])]
    run(args)


def manifest_row(fixture):
    """The oracle row of one fixture, without the generator-only track table."""
    keys = ("name", "original", "anime", "expect", "warnings", "needs_remux")
    return {key: fixture[key] for key in keys}


def main(argv):
    """Entry point; returns the process exit code."""
    if len(argv) not in (2, 3) or (len(argv) == 3 and argv[2] != "--large"):
        print(__doc__, file=sys.stderr)
        return 2
    if shutil.which("ffmpeg") is None:
        print("track_fixtures: ffmpeg not on PATH", file=sys.stderr)
        return 1
    out_dir = pathlib.Path(argv[1])
    out_dir.mkdir(parents=True, exist_ok=True)
    work = out_dir / ".work"
    work.mkdir(exist_ok=True)
    large = len(argv) == 3
    fixtures = FIXTURES[:1] if large else FIXTURES
    try:
        side = side_files(work)
        for fixture in fixtures:
            build(fixture, out_dir, LARGE if large else SMALL, side)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        detail = getattr(exc, "stderr", b"") or b""
        print(f"track_fixtures: ffmpeg failed: {exc}: {detail.decode(errors='replace')[:2000]}", file=sys.stderr)
        return 1
    manifest = {"policy": "praetor-decided-media-track-policy",
                "fixtures": [manifest_row(f) for f in fixtures]}
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
