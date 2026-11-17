# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Measure pipeline backend options for ADR-0002 on this host.

Compares, for the same input and settings, the direct codec command-line
tools (SvtAv1EncApp, x265, dav1d), FFmpeg's libav* wrappers (ffmpeg,
ffprobe) and GStreamer (gst-launch-1.0, gst-discoverer-1.0), plus mkvmerge
for remuxing. Each case runs pinned to the same CPU set, so the numbers
compare backends rather than core counts. CPU time (user + system of the
child) is the primary figure: it is robust against other load on a shared
host, which wall time is not.

Usage:
    python3 -I tools/bench/backends.py <work-dir> <out.json>
        [--inproc-probe <binary>] [--inproc-remux <binary>]

<work-dir> receives the generated inputs (about 1 GiB). --inproc-probe names
a binary that probes every path given to it in one process through
libavformat; --inproc-remux names a binary called as
`<binary> <in> <out> <comma-separated stream indices>` that stream-copies
through libavformat in process. The ADR evidence carries the source of the
ones used. Exit 0 when every case ran, 1 when a case failed, 2 on a usage
error.
"""

import json
import os
import pathlib
import platform
import statistics
import subprocess
import sys
import time

CASE_TIMEOUT_S = 900  # HISS-02: deadline per measured command
POLL_S = 0.02
MAX_POLLS = int(CASE_TIMEOUT_S / POLL_S) + 1
CPUS = "0-7"  # every case gets the same eight logical CPUs
REPEATS = 3
FIXTURES = pathlib.Path(__file__).resolve().parent.parent / "fixtures" / "track_fixtures.py"


def measure(args, stdout_path=None):
    """Run one command pinned to CPUS; return wall, cpu seconds and peak RSS in MiB."""
    sink = open(stdout_path, "wb") if stdout_path else subprocess.DEVNULL
    try:
        start = time.monotonic()
        child = subprocess.Popen(["taskset", "-c", CPUS] + args, stdout=sink,
                                 stderr=subprocess.PIPE)
        for _ in range(MAX_POLLS):
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                break
            time.sleep(POLL_S)
        else:
            child.kill()
            child.wait(timeout=10)
            raise RuntimeError(f"timeout after {CASE_TIMEOUT_S} s: {args}")
        wall = time.monotonic() - start
        child.returncode = os.waitstatus_to_exitcode(status)
        err = child.stderr.read().decode(errors="replace")
        child.stderr.close()
    finally:
        if stdout_path:
            sink.close()
    if child.returncode != 0:
        raise RuntimeError(f"exit {child.returncode}: {args}: {err[-1500:]}")
    return {"wall_s": wall, "cpu_s": usage.ru_utime + usage.ru_stime,
            "max_rss_mib": usage.ru_maxrss / 1024}


def repeat(name, args, stdout_path=None, repeats=REPEATS):
    """Median of repeated runs of one case."""
    runs = [measure(args, stdout_path) for _ in range(repeats)]
    keys = ("wall_s", "cpu_s", "max_rss_mib")
    row = {key: round(statistics.median(r[key] for r in runs), 3) for key in keys}
    row["case"] = name
    row["runs"] = repeats
    return row


def prepare(work):
    """Generate the small fixture set, the large remux input and the raw encode input."""
    small = work / "small"
    large = work / "large"
    for out_dir, extra in ((small, []), (large, ["--large"])):
        if not (out_dir / "manifest.json").exists():
            subprocess.run([sys.executable, "-I", str(FIXTURES), str(out_dir)] + extra,
                           check=True, timeout=CASE_TIMEOUT_S)
    raw = work / "src_1080p_240f.y4m"
    if not raw.exists():
        subprocess.run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-f", "lavfi", "-i",
                        "testsrc2=size=1920x1080:rate=24", "-frames:v", "240",
                        "-pix_fmt", "yuv420p", str(raw)], check=True, timeout=CASE_TIMEOUT_S)
    return small, large / "f01_movie_en_de_extra.mkv", raw


def probe_cases(small, inproc):
    """M1: per-file probe cost, process per file versus one in-process probe of all files."""
    files = sorted(str(p) for p in small.glob("f*.m*"))
    rows = []
    script = ["sh", "-c", 'for f in "$@"; do ffprobe -v error -show_streams -of json "$f"; done', "sh"]
    rows.append(repeat("M1 ffprobe, one process per file", script + files, repeats=5))
    gst = ["sh", "-c", 'for f in "$@"; do gst-discoverer-1.0 "$f"; done', "sh"]
    rows.append(repeat("M1 gst-discoverer, one process per file", gst + files, repeats=5))
    if inproc:
        rows.append(repeat("M1 libavformat in process (ffmpeg-next)", [inproc] + files, repeats=5))
    for row in rows:
        row["files"] = len(files)
    return rows


def remux_cases(src, work, inproc):
    """M2: stream-copy remux of the 1080p 120 s input, keeping the policy's tracks."""
    out = work / "remux"
    out.mkdir(exist_ok=True)
    kept = [0, 1, 2, 5, 6, 7, 8, 9, 12]
    keep = [arg for index in kept for arg in ("-map", f"0:{index}")]
    ff = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src)] + keep
    rows = [repeat("M2 ffmpeg -c copy, policy tracks", ff + ["-c", "copy", str(out / "ff.mkv")])]
    if inproc:
        rows.append(repeat("M2 libavformat in process, policy tracks",
                           [inproc, str(src), str(out / "inproc.mkv"), ",".join(map(str, kept))]))
    mkv = ["mkvmerge", "-q", "-o", str(out / "mkvmerge.mkv"), "--audio-tracks", "1,2",
           "--subtitle-tracks", "5,6,7,8,9", str(src)]
    rows.append(repeat("M2 mkvmerge, policy tracks", mkv))
    ffav = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-map", "0:0",
            "-map", "0:1", "-map", "0:2", "-c", "copy", str(out / "ff_av.mkv")]
    rows.append(repeat("M2 ffmpeg -c copy, video + 2 audio", ffav))
    gst = ["gst-launch-1.0", "-q", "filesrc", f"location={src}", "!", "matroskademux", "name=d",
           "matroskamux", "name=m", "!", "filesink", f"location={out / 'gst_av.mkv'}",
           "d.video_0", "!", "queue", "!", "h264parse", "!", "m.",
           "d.audio_0", "!", "queue", "!", "aacparse", "!", "m.",
           "d.audio_1", "!", "queue", "!", "aacparse", "!", "m."]
    rows.append(repeat("M2 gst-launch remux, video + 2 audio", gst))
    return rows


def encode_cases(raw, work):
    """M3/M4: the same encode and decode through the direct tool, ffmpeg and GStreamer."""
    out = work / "enc"
    out.mkdir(exist_ok=True)
    ff = ["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(raw)]
    rows = [
        repeat("M3 AV1 SvtAv1EncApp preset 8 crf 35", ["SvtAv1EncApp", "--preset", "8", "--crf", "35",
               "-i", str(raw), "-b", str(out / "svt_cli.ivf"), "--progress", "0"]),
        repeat("M3 AV1 ffmpeg libsvtav1 preset 8 crf 35", ff + ["-c:v", "libsvtav1", "-preset", "8",
               "-crf", "35", str(out / "svt_ff.ivf")]),
        repeat("M4 AV1 gst svtav1enc preset 8 crf 35", ["gst-launch-1.0", "-q", "filesrc",
               f"location={raw}", "!", "y4mdec", "!", "svtav1enc", "preset=8", "crf=35", "!",
               "av1parse", "!", "matroskamux", "!", "filesink", f"location={out / 'svt_gst.mkv'}"]),
        repeat("M3 HEVC x265 preset fast crf 28", ["x265", "--preset", "fast", "--crf", "28",
               "--log-level", "error", "--input", str(raw), "-o", str(out / "x265_cli.hevc")]),
        repeat("M3 HEVC ffmpeg libx265 preset fast crf 28", ff + ["-c:v", "libx265", "-preset", "fast",
               "-crf", "28", "-x265-params", "log-level=error", str(out / "x265_ff.hevc")]),
        repeat("M4 HEVC gst x265enc preset fast crf 28", ["gst-launch-1.0", "-q", "filesrc",
               f"location={raw}", "!", "y4mdec", "!", "x265enc", "speed-preset=fast",
               "option-string=crf=28", "!", "h265parse", "!", "matroskamux", "!", "filesink",
               f"location={out / 'x265_gst.mkv'}"]),
    ]
    ivf = str(out / "svt_cli.ivf")
    rows.append(repeat("M3 AV1 decode dav1d", ["dav1d", "-q", "--threads", "8", "-i", ivf,
                                                "--muxer", "null", "-o", "/dev/null"]))
    rows.append(repeat("M3 AV1 decode ffmpeg libdav1d", ["ffmpeg", "-nostdin", "-loglevel", "error",
                       "-threads", "8", "-c:v", "libdav1d", "-i", ivf, "-f", "null", "-"]))
    return rows


def sizes(work):
    """Output sizes, so a speed figure is never read without the bytes it produced."""
    found = sorted((work / "enc").glob("*")) + sorted((work / "remux").glob("*"))
    return {p.name: p.stat().st_size for p in found}


def host_facts():
    """Host and tool versions the numbers depend on."""
    def first_line(args):
        result = subprocess.run(args, capture_output=True, timeout=60, check=False)
        text = (result.stdout or result.stderr).decode(errors="replace").strip()
        return text.splitlines()[0] if text else ""
    return {"cpu_set": CPUS, "nproc": os.cpu_count(), "kernel": platform.release(),
            "loadavg_start": os.getloadavg(),
            "ffmpeg": first_line(["ffmpeg", "-hide_banner", "-version"]),
            "svt": first_line(["SvtAv1EncApp", "--version"]),
            "x265": first_line(["x265", "--version"]),
            "dav1d": first_line(["dav1d", "--version"]),
            "gstreamer": first_line(["gst-launch-1.0", "--version"]),
            "mkvmerge": first_line(["mkvmerge", "--version"])}


def main(argv):
    """Entry point; returns the process exit code."""
    options = dict(zip(argv[3::2], argv[4::2]))
    if len(argv) < 3 or len(argv) % 2 == 0 or set(options) - {"--inproc-probe", "--inproc-remux"}:
        print(__doc__, file=sys.stderr)
        return 2
    work = pathlib.Path(argv[1])
    work.mkdir(parents=True, exist_ok=True)
    facts = host_facts()
    try:
        small, large, raw = prepare(work)
        rows = (probe_cases(small, options.get("--inproc-probe"))
                + remux_cases(large, work, options.get("--inproc-remux"))
                + encode_cases(raw, work))
    except (RuntimeError, subprocess.SubprocessError, OSError) as exc:
        print(f"backends: {exc}", file=sys.stderr)
        return 1
    facts["loadavg_end"] = os.getloadavg()
    result = {"host": facts, "rows": rows, "sizes_bytes": sizes(work)}
    pathlib.Path(argv[2]).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    for row in rows:
        print(f"{row['case']:<48} wall {row['wall_s']:>8.3f} s  cpu {row['cpu_s']:>8.3f} s  "
              f"rss {row['max_rss_mib']:>7.1f} MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
