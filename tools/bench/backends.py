# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""
Measure pipeline backend options for ADR-0002 on this host.

Compares, for the same input and settings, the direct codec command-line
tools (SvtAv1EncApp, x265, dav1d), FFmpeg's libav* wrappers (ffmpeg,
ffprobe) and GStreamer (gst-launch-1.0, gst-discoverer-1.0), plus mkvmerge
for remuxing. Each case runs pinned to the same CPU set, so the numbers
compare backends rather than core counts. CPU time (user + system of the
child) is the primary figure: it is robust against other load on a shared
host, which wall time is not.

Usage:
    python3 -I tools/bench/backends.py <work-dir> <out.json>
        [--inproc-probe <binary>] [--inproc-remux <binary>] [--cases all|probe]

<work-dir> receives the generated inputs (about 1 GiB). --inproc-probe names
a binary that probes every path given to it in one process through
libavformat; --inproc-remux names a binary called as
`<binary> <in> <out> <comma-separated stream indices>` that stream-copies
through libavformat in process. The ADR evidence carries the source of the
ones used. --cases probe runs only the probe rows. One run at a time holds
<work-dir>/.bench.lock; a second run on the same directory fails at once.
Exit 0 when every case ran, 1 when a case failed, 2 on a usage error.
"""

import contextlib
import fcntl
import hashlib
import json
import os
import pathlib
import platform
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

CASE_TIMEOUT_S = 900  # HISS-02: deadline per measured command
POLL_S = 0.02
MAX_POLLS = int(CASE_TIMEOUT_S / POLL_S) + 1
CPUS = "0-7"  # every case gets the same eight logical CPUs
REPEATS = 3
RAW_FRAMES = 240
RAW_FRAME_BYTES = 1920 * 1080 * 3 // 2 + len(b"FRAME\n")  # one 4:2:0 8-bit Y4M frame
Y4M_HEADER_MAX = 256  # bytes read for the stream header line; ffmpeg writes about 60
FIXTURES = pathlib.Path(__file__).resolve().parent.parent / "fixtures" / "track_fixtures.py"


def tool(args):
    """Resolve the program of one argument list to an absolute path; raise when it is missing."""
    exe = shutil.which(args[0])
    if exe is None:
        raise RuntimeError(f"not on PATH: {args[0]}")
    return [exe] + list(args[1:])


def stop_group(child):
    """Kill the timed-out child's whole process group, then reap the child."""
    pgid = os.getpgid(child.pid)
    if pgid > 1 and pgid == child.pid:  # HISS-08: never signal group 0, 1 or our own
        os.killpg(pgid, signal.SIGKILL)
    child.wait(timeout=10)


def measure(args, stdout_path=None):
    """
    Run one command pinned to CPUS; return wall, cpu seconds and peak RSS in MiB.

    The command runs in its own session, so a timeout kills every process it
    started. stderr goes to an anonymous file, so a chatty command never
    blocks on a full pipe.
    """
    with contextlib.ExitStack() as stack:
        sink = stack.enter_context(open(stdout_path, "wb")) if stdout_path else subprocess.DEVNULL
        err_file = stack.enter_context(tempfile.TemporaryFile())
        argv = tool(["taskset", "-c", CPUS] + tool(args))
        start = time.monotonic()
        child = subprocess.Popen(argv, stdout=sink, stderr=err_file, start_new_session=True)
        for _ in range(MAX_POLLS):
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                break
            time.sleep(POLL_S)
        else:
            stop_group(child)
            raise RuntimeError(f"timeout after {CASE_TIMEOUT_S} s: {args}")
        wall = time.monotonic() - start
        child.returncode = os.waitstatus_to_exitcode(status)
        err_file.seek(0)
        err = err_file.read().decode(errors="replace")
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


def lock_work(work):
    """Take the work directory's lock for the whole run; raise when another run holds it."""
    path = work / ".bench.lock"
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        raise RuntimeError(f"another run holds {path}") from None
    return fd


def bare_name(name):
    """Whether name is one plain file name: a non-empty string, no directory part, not `.` or `..`."""
    return (isinstance(name, str) and name not in ("", ".", "..")
            and "/" not in name and "\0" not in name and pathlib.PurePath(name).name == name)


def fixture_names(out_dir):
    """Files of the fixture set published in out_dir, or None when it is absent or stale."""
    manifest = out_dir / "manifest.json"
    if manifest.is_symlink() or not manifest.is_file():
        return None
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
    except ValueError:
        return None
    current = hashlib.sha256(FIXTURES.read_bytes()).hexdigest()
    if not isinstance(data, dict) or data.get("generator_sha256") != current:
        return None
    rows = data.get("fixtures")
    if not isinstance(rows, list) or not rows:
        return None
    names = [row.get("name") if isinstance(row, dict) else None for row in rows]
    if not all(bare_name(name) for name in names):
        return None
    return [str(out_dir / name) for name in names]


def complete_y4m(path):
    """
    Whether path is a whole cached raw input: one 1080p 4:2:0 header, RAW_FRAMES frames, nothing more.

    The size must equal the header plus RAW_FRAMES full frames exactly, and every frame must start with
    its FRAME marker, so a file cut anywhere (header, marker or payload) is regenerated, never reused.
    """
    if path.is_symlink() or not path.is_file():
        return False
    with path.open("rb") as stream:
        header = stream.readline(Y4M_HEADER_MAX)
        tokens = header.split()
        if not header.endswith(b"\n") or tokens[:1] != [b"YUV4MPEG2"]:
            return False
        if b"W1920" not in tokens or b"H1080" not in tokens:
            return False
        if any(t.startswith(b"C") and not t.startswith(b"C420") for t in tokens[1:]):
            return False
        if path.stat().st_size != len(header) + RAW_FRAMES * RAW_FRAME_BYTES:
            return False
        for frame in range(RAW_FRAMES):
            stream.seek(len(header) + frame * RAW_FRAME_BYTES)
            if stream.read(len(b"FRAME\n")) != b"FRAME\n":
                return False
    return True


def raw_input(work):
    """The raw encode input; generated under a temporary name and renamed once complete."""
    raw = work / f"src_1080p_{RAW_FRAMES}f.y4m"
    if complete_y4m(raw):
        return raw
    partial = work / (raw.name + ".partial")
    try:
        subprocess.run(tool(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-f", "lavfi", "-i",
                             "testsrc2=size=1920x1080:rate=24", "-frames:v", str(RAW_FRAMES),
                             "-pix_fmt", "yuv420p", "-f", "yuv4mpegpipe", str(partial)]),
                       check=True, timeout=CASE_TIMEOUT_S)
    except (subprocess.SubprocessError, OSError):
        partial.unlink(missing_ok=True)
        raise
    os.replace(partial, raw)
    return raw


def fixture_set(out_dir, extra):
    """Regenerate out_dir unless it holds a set from the current generator."""
    if fixture_names(out_dir) is None:
        subprocess.run(tool([sys.executable, "-I", str(FIXTURES), str(out_dir)] + extra),
                       check=True, timeout=CASE_TIMEOUT_S)
    return out_dir


def probe_cases(small, inproc):
    """
    M1: per-file probe cost, process per file versus one in-process probe of all files.

    The in-process probe prints index, medium, codec id and language per
    stream; the second ffprobe row prints the same four fields, so it does the
    same work, while the first row also serialises every other stream field.
    """
    files = fixture_names(small)
    if not files:
        raise RuntimeError(f"no current fixture manifest in {small}")
    each = 'for f in "$@"; do {} "$f" || exit 1; done'
    rows = []
    full = each.format("ffprobe -v error -show_streams -of json")
    rows.append(repeat("M1 ffprobe, one process per file",
                       ["sh", "-c", full, "sh"] + files, repeats=5))
    four = each.format("ffprobe -v error -show_entries "
                       "stream=index,codec_type,codec_name:stream_tags=language -of compact")
    rows.append(repeat("M1 ffprobe four fields, one process per file",
                       ["sh", "-c", four, "sh"] + files, repeats=5))
    gst = ["sh", "-c", each.format("gst-discoverer-1.0"), "sh"]
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
        result = subprocess.run(tool(args), capture_output=True, timeout=60, check=False)
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
    options = dict(zip(argv[3::2], argv[4::2], strict=False))
    known = {"--inproc-probe", "--inproc-remux", "--cases"}
    cases = options.get("--cases", "all")
    if len(argv) < 3 or len(argv) % 2 == 0 or set(options) - known or cases not in ("all", "probe"):
        print(__doc__, file=sys.stderr)
        return 2
    work = pathlib.Path(argv[1])
    work.mkdir(parents=True, exist_ok=True)
    try:
        lock = lock_work(work)
        facts = host_facts()
        rows = probe_cases(fixture_set(work / "small", []), options.get("--inproc-probe"))
        if cases == "all":
            large = fixture_set(work / "large", ["--large"]) / "f01_movie_en_de_extra.mkv"
            rows += remux_cases(large, work, options.get("--inproc-remux"))
            rows += encode_cases(raw_input(work), work)
    except (RuntimeError, subprocess.SubprocessError, OSError) as exc:
        print(f"backends: {exc}", file=sys.stderr)
        return 1
    facts["loadavg_end"] = os.getloadavg()
    result = {"host": facts, "cases": cases, "rows": rows,
              "sizes_bytes": sizes(work) if cases == "all" else {}}
    pathlib.Path(argv[2]).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    os.close(lock)
    for row in rows:
        print(f"{row['case']:<48} wall {row['wall_s']:>8.3f} s  cpu {row['cpu_s']:>8.3f} s  "
              f"rss {row['max_rss_mib']:>7.1f} MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
