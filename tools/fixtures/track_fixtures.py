# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Generate the synthetic track-policy fixture set and its expected plans."""

# subprocess runs argument lists only, never a shell (run).
import hashlib
import json
import os
import pathlib
import secrets
import shutil
import subprocess  # nosec B404  # FIXTURES argument lists, no shell
import sys
import tempfile

USAGE = """\
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
The set is built in a new staging directory beside <out-dir> and swapped
in only when every file and the manifest are written, so a failed or
interrupted run leaves the previous set as it was. If the swap fails and the
previous set cannot be moved back, it stays in the `.<name>.old-*` directory
beside <out-dir> and the error names that directory. An existing <out-dir> is
replaced only when it is empty or holds a manifest of this generator; the
manifest names the generator's SHA-256, so a reader can tell a stale set.
Requires the ffmpeg binary on PATH. Exit 0 on success, 2 on a usage error,
1 when an ffmpeg call fails or times out or <out-dir> cannot be replaced.
"""

FFMPEG_TIMEOUT_S = 600  # HISS-02: deadline for one ffmpeg call
MAX_CHAPTERS = 1000
SMALL = {"size": "320x180", "seconds": 2}
LARGE = {"size": "1920x1080", "seconds": 120}
POLICY = "praetor-decided-media-track-policy"
KIND_MAP = {"a": "audio", "s": "subtitle", "t": "attachment", "c": "video"}

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
        "name": "f02_anime_ja.mkv", "original": "ja", "anime": True, "chapters": 3,
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
    exe = shutil.which(args[0])
    if exe is None:
        raise FileNotFoundError(f"{args[0]} not on PATH")
    # args are FIXTURES constants of this file; the program is resolved by name and no shell runs, so the
    # bandit and semgrep subprocess audit rules do not apply on this line.
    argv = [exe] + args[1:]
    # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit  # FIXTURES argument list, no shell
    subprocess.run(argv, check=True, timeout=FFMPEG_TIMEOUT_S, stdout=subprocess.DEVNULL,  # nosec B603  # FIXTURES argument list, no shell
                   stderr=subprocess.PIPE)


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


def chapter_metadata(seconds, chapters):
    """Build an FFMETADATA1 document with `chapters` evenly spaced millisecond ranges."""
    if chapters < 0 or chapters > MAX_CHAPTERS:
        raise ValueError(f"chapter count out of bounds: {chapters}")
    total_ms = int(seconds) * 1000
    lines = [";FFMETADATA1\n"]
    for index in range(chapters):
        start = (index * total_ms) // chapters
        end = ((index + 1) * total_ms) // chapters
        lines.append(
            f"[CHAPTER]\nTIMEBASE=1/1000\nSTART={start}\nEND={end}\ntitle=Chapter {index + 1}\n"
        )
    return "".join(lines)


def count_streams(kinds):
    """Count stream kinds in fixed dictionary key order."""
    counts = {"video": 0, "audio": 0, "subtitle": 0, "attachment": 0}
    for kind in kinds:
        counts[kind] += 1
    return counts


def stream_counts(fixture):
    """Compute source and expected output per-kind stream counts from track definitions."""
    all_kinds = ["video"] + [KIND_MAP[track[0]] for track in fixture["tracks"]]
    kept_kinds = [all_kinds[index] for index in fixture["expect"]]
    return {"source": count_streams(all_kinds), "output": count_streams(kept_kinds)}


def build(fixture, out_dir, geometry, side):
    """Generate one fixture file."""
    srt, fonts, cover = side
    args = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error"]
    inputs = input_args(fixture, geometry, srt)
    chapters = fixture.get("chapters", 0)
    if chapters > 0:
        ffmeta = srt.parent / f"{fixture['name']}.ffmeta"
        ffmeta.write_text(
            chapter_metadata(geometry["seconds"], chapters), encoding="utf-8"
        )
        chapter_input = inputs.count("-i")
        inputs += ["-f", "ffmetadata", "-i", str(ffmeta)]
        chapter_args = ["-map_chapters", str(chapter_input)]
    else:
        chapter_args = ["-map_chapters", "-1"]
    args += inputs
    args += stream_args(fixture, fonts, cover)
    args += chapter_args
    args += codec_args(fixture)
    # -t, not -shortest: the subtitle inputs end at 1.5 s and would cut the file.
    args += ["-t", str(geometry["seconds"]), str(out_dir / fixture["name"])]
    run(args)


def manifest_row(fixture):
    """The oracle row of one fixture, without the generator-only track table."""
    keys = ("name", "original", "anime", "expect", "warnings", "needs_remux")
    row = {key: fixture[key] for key in keys}
    row["chapters"] = fixture.get("chapters", 0)
    row["counts"] = stream_counts(fixture)
    return row


def replaceable(out_dir):
    """True when out_dir is absent, empty, or a real directory holding this generator's manifest."""
    if not os.path.lexists(out_dir):
        return True
    if out_dir.is_symlink() or not out_dir.is_dir():
        return False
    if next(out_dir.iterdir(), None) is None:
        return True
    manifest = out_dir / "manifest.json"
    if manifest.is_symlink() or not manifest.is_file():
        return False
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    return isinstance(data, dict) and data.get("policy") == POLICY


def write_manifest(staging, fixtures):
    """Write the manifest last; mode "x" creates a new file and fails on any existing path."""
    manifest = {"policy": POLICY,
                "generator_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                "fixtures": [manifest_row(f) for f in fixtures]}
    with open(staging / "manifest.json", "x", encoding="utf-8") as handle:
        handle.write(json.dumps(manifest, indent=2) + "\n")


def restore(previous, out_dir, holder, cause):
    """Put the previous set back after a failed swap, then drop the holder only if it is empty."""
    # When the set cannot be moved back, or the holder still holds anything, the holder stays on disk and
    # the error names it, so a failed publish never deletes the previous set.
    try:
        if os.path.lexists(previous) and not os.path.lexists(out_dir):
            os.rename(previous, out_dir)
        os.rmdir(holder)  # removes only an empty directory
    except OSError as err:
        raise OSError(f"publish failed and the previous set stays under {holder}: {err}") from cause


def make_staging(out_dir):
    """Create a new staging directory beside out_dir; os.mkdir applies the umask like any user mkdir."""
    staging = out_dir.parent / f".{out_dir.name}.tmp-{os.getpid()}-{secrets.token_hex(8)}"
    os.mkdir(staging)  # raises FileExistsError on a name clash: never reuses a directory
    return staging


def publish(staging, out_dir):
    """Swap the finished staging directory in for out_dir, then delete the previous set."""
    holder = pathlib.Path(tempfile.mkdtemp(prefix=f".{out_dir.name}.old-", dir=out_dir.parent))
    previous = holder / "previous"
    try:
        if os.path.lexists(out_dir):
            os.rename(out_dir, previous)
            if not replaceable(previous):
                raise OSError(f"refusing {out_dir}: displaced set is no longer replaceable")
        os.rename(staging, out_dir)
    except BaseException as exc:
        restore(previous, out_dir, holder, exc)
        raise
    shutil.rmtree(holder)


def generate(staging, fixtures, geometry):
    """Build every fixture and the manifest inside the staging directory."""
    work = staging / ".work"
    work.mkdir()
    side = side_files(work)
    for fixture in fixtures:
        build(fixture, staging, geometry, side)
    shutil.rmtree(work)
    write_manifest(staging, fixtures)


def main(argv):
    """Entry point; returns the process exit code."""
    if len(argv) not in (2, 3) or (len(argv) == 3 and argv[2] != "--large"):
        print(__doc__ + "\n\n" + USAGE, file=sys.stderr)
        return 2
    if shutil.which("ffmpeg") is None:
        print("track_fixtures: ffmpeg not on PATH", file=sys.stderr)
        return 1
    out_dir = pathlib.Path(argv[1]).absolute()
    if not replaceable(out_dir):
        print(f"track_fixtures: refusing {out_dir}: a symlink, not a directory, or a "
              "non-empty directory without a manifest of this generator", file=sys.stderr)
        return 1
    large = len(argv) == 3
    out_dir.parent.mkdir(parents=True, exist_ok=True)
    staging = make_staging(out_dir)
    try:
        generate(staging, FIXTURES[:1] if large else FIXTURES, LARGE if large else SMALL)
        publish(staging, out_dir)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        detail = getattr(exc, "stderr", b"") or b""
        print(f"track_fixtures: ffmpeg failed: {exc}: {detail.decode(errors='replace')[:2000]}",
              file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"track_fixtures: {exc}", file=sys.stderr)
        return 1
    finally:
        if os.path.lexists(staging):
            shutil.rmtree(staging)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
