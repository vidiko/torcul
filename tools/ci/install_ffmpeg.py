# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Build and install pinned FFmpeg 9.0.2 from source for CI and local verification."""

import hashlib
import os
import pathlib
import re
import shutil
import subprocess  # nosec B404  # nosemgrep
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request

FFMPEG_VERSION = "9.0.2"
FFMPEG_TAG = "n9.0.2"
FFMPEG_URL = f"https://ffmpeg.org/releases/ffmpeg-{FFMPEG_VERSION}.tar.xz"
FFMPEG_SHA256 = "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
STAMP_NAME = ".torcul-ffmpeg-sha256"

NETWORK_TIMEOUT_S = 60
BUILD_TIMEOUT_S = 600
VERSION_TIMEOUT_S = 30
CHUNK_BYTES = 65536
MAX_CHUNKS = 2048
MAX_TAR_MEMBERS = 20000
MAX_BUILD_JOBS = 8

CONFIGURE_FLAGS = (
    "--enable-shared",
    "--disable-static",
    "--enable-gpl",
    "--enable-libx264",
    "--enable-zlib",
    "--disable-doc",
    "--disable-debug",
    "--disable-network",
    "--disable-autodetect",
    "--disable-programs",
    "--enable-ffmpeg",
    "--disable-everything",
    "--enable-avdevice",
    "--enable-avfilter",
    "--enable-swscale",
    "--enable-swresample",
    "--enable-indev=lavfi",
    "--enable-protocol=file,pipe",
    "--enable-demuxer=matroska,mov,srt,ass,ffmetadata,image2",
    "--enable-muxer=matroska,mp4,mov,srt,ass,ffmetadata,image2,mjpeg",
    "--enable-decoder=h264,aac,mjpeg,subrip,srt,ass,ssa,movtext,mov_text,text,rawvideo,pcm_s16le,wrapped_avframe",
    "--enable-encoder=libx264,aac,mjpeg,subrip,srt,ass,ssa,movtext,mov_text,rawvideo,pcm_s16le,wrapped_avframe",
    "--enable-parser=h264,aac,mjpeg",
    "--enable-bsf=aac_adtstoasc,extract_extradata",
    "--enable-filter=testsrc2,sine,scale,format,aresample,null,anull,abuffer,buffer,abuffersink,buffersink",
)


def parse_args(argv):
    """Parse `<prefix> [--sha256 <64-hex>]` command arguments."""
    if len(argv) == 2:
        return pathlib.Path(argv[1]).resolve(), FFMPEG_SHA256
    if len(argv) == 4 and argv[2] == "--sha256":
        digest = argv[3].strip().lower()
        if not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError("--sha256 requires 64 lowercase hexadecimal digits")
        return pathlib.Path(argv[1]).resolve(), digest
    raise ValueError(f"usage: {argv[0]} <prefix> [--sha256 <64-hex>]")


def run_checked(args, *, cwd=None, env=None, timeout=BUILD_TIMEOUT_S):
    """Run one resolved executable argument list under an explicit deadline."""
    exe = args[0] if os.path.isabs(args[0]) else shutil.which(args[0])
    if exe is None:
        raise FileNotFoundError(f"required executable not on PATH: {args[0]}")
    argv = [exe, *args[1:]]
    return subprocess.run(  # nosec B603  # nosemgrep
        argv,
        cwd=cwd,
        env=env,
        check=True,
        timeout=timeout,
        capture_output=True,
    )


def runtime_env(prefix):
    """Build an environment mapping pointing at `prefix` libraries and binaries."""
    env = dict(os.environ)
    lib_dir = str(prefix / "lib")
    bin_dir = str(prefix / "bin")
    pc_dir = str(prefix / "lib" / "pkgconfig")
    env["PATH"] = f"{bin_dir}:{env['PATH']}" if env.get("PATH") else bin_dir
    env["PKG_CONFIG_PATH"] = (
        f"{pc_dir}:{env['PKG_CONFIG_PATH']}" if env.get("PKG_CONFIG_PATH") else pc_dir
    )
    env["LD_LIBRARY_PATH"] = (
        f"{lib_dir}:{env['LD_LIBRARY_PATH']}"
        if env.get("LD_LIBRARY_PATH")
        else lib_dir
    )
    return env


def expected_stamp(expected_sha256):
    """Bind the install stamp to both the source SHA-256 and configure flags."""
    cfg_hash = hashlib.sha256("\n".join(CONFIGURE_FLAGS).encode("utf-8")).hexdigest()
    return f"{expected_sha256}:{cfg_hash}"


def verify_installed(prefix, expected_sha256):
    """Check stamp, pkg-config metadata and `ffmpeg -version` output inside `prefix`."""
    stamp = prefix / STAMP_NAME
    pc_file = prefix / "lib" / "pkgconfig" / "libavformat.pc"
    ffmpeg_bin = prefix / "bin" / "ffmpeg"
    if not (stamp.is_file() and pc_file.is_file() and ffmpeg_bin.is_file()):
        return False
    if stamp.read_text(encoding="utf-8").strip() != expected_stamp(expected_sha256):
        return False
    result = run_checked(
        [str(ffmpeg_bin), "-version"],
        env=runtime_env(prefix),
        timeout=VERSION_TIMEOUT_S,
    )
    first_line = result.stdout.decode("utf-8", errors="replace").splitlines()[0]
    return FFMPEG_TAG in first_line


def download_and_verify(url, archive_path, expected_sha256):
    """Download `url` under a network timeout and verify its SHA-256 digest."""
    digest = hashlib.sha256()
    finished = False
    with (
        urllib.request.urlopen(url, timeout=NETWORK_TIMEOUT_S) as response,  # nosec B310
        open(archive_path, "wb") as handle,
    ):
        for _ in range(MAX_CHUNKS):
            chunk = response.read(CHUNK_BYTES)
            if not chunk:
                finished = True
                break
            digest.update(chunk)
            handle.write(chunk)
    if not finished:
        raise ValueError(f"download exceeded {MAX_CHUNKS * CHUNK_BYTES} byte bound")
    actual = digest.hexdigest()
    if actual != expected_sha256:
        raise ValueError(
            f"FFmpeg SHA-256 mismatch: expected {expected_sha256}, got {actual}"
        )


def safe_extract(archive_path, dest_dir):
    """Validate and unpack a `.tar.xz` archive without links, devices or traversal."""
    dest_dir.mkdir(parents=True, exist_ok=True)
    root = dest_dir.resolve()
    with tarfile.open(archive_path, mode="r:xz") as tar:
        for index, member in enumerate(tar):
            if index >= MAX_TAR_MEMBERS:
                raise ValueError("archive member bound exceeded")
            if member.issym() or member.islnk() or member.isdev():
                raise ValueError(f"unsupported archive entry type: {member.name}")
            target = (root / member.name).resolve()
            if not target.is_relative_to(root):
                raise ValueError(f"archive path escapes destination: {member.name}")
            tar.extract(member, path=root, filter="data")


def build_from_source(src_dir, prefix):
    """Configure, compile and install FFmpeg 9.0.2 with bounded parallelism."""
    release = (src_dir / "RELEASE").read_text(encoding="utf-8").strip()
    if release != FFMPEG_VERSION:
        raise ValueError(f"unexpected FFmpeg RELEASE {release!r}")
    (src_dir / "VERSION").write_text(f"{FFMPEG_TAG}\n", encoding="utf-8")
    configure = str(src_dir / "configure")
    rpath_flag = f"--extra-ldflags=-Wl,-rpath,{prefix / 'lib'}"
    run_checked(
        [configure, f"--prefix={prefix}", rpath_flag, *CONFIGURE_FLAGS], cwd=src_dir
    )
    jobs = str(max(1, min(os.cpu_count() or 2, MAX_BUILD_JOBS)))
    run_checked(["make", f"-j{jobs}"], cwd=src_dir)
    run_checked(["make", "install"], cwd=src_dir)


def prepare_prefix(prefix):
    """Ensure `prefix` is absent, empty or owned by a previous run of this installer."""
    if not os.path.lexists(prefix):
        return
    if prefix.is_symlink() or not prefix.is_dir():
        raise OSError(f"refusing non-directory prefix: {prefix}")
    if next(prefix.iterdir(), None) is not None and not (prefix / STAMP_NAME).is_file():
        raise OSError(f"refusing unowned non-empty prefix: {prefix}")
    shutil.rmtree(prefix)


def write_github_env(prefix, env_file_path):
    """Append PATH, PKG_CONFIG_PATH and LD_LIBRARY_PATH for subsequent CI steps."""
    merged = runtime_env(prefix)
    lines = [
        f"PATH={merged['PATH']}\n",
        f"PKG_CONFIG_PATH={merged['PKG_CONFIG_PATH']}\n",
        f"LD_LIBRARY_PATH={merged['LD_LIBRARY_PATH']}\n",
    ]
    with open(env_file_path, "a", encoding="utf-8") as handle:
        handle.writelines(lines)


def install(prefix, expected_sha256):
    """Install or reuse pinned FFmpeg at `prefix`, cleaning up any failed prefix."""
    if not (prefix.is_dir() and verify_installed(prefix, expected_sha256)):
        prepare_prefix(prefix)
        with tempfile.TemporaryDirectory() as work:
            work_dir = pathlib.Path(work)
            archive = work_dir / f"ffmpeg-{FFMPEG_VERSION}.tar.xz"
            download_and_verify(FFMPEG_URL, archive, expected_sha256)
            extract_dir = work_dir / "src"
            safe_extract(archive, extract_dir)
            src_dir = extract_dir / f"ffmpeg-{FFMPEG_VERSION}"
            try:
                prefix.parent.mkdir(parents=True, exist_ok=True)
                build_from_source(src_dir, prefix)
                (prefix / STAMP_NAME).write_text(
                    f"{expected_stamp(expected_sha256)}\n", encoding="utf-8"
                )
                if not verify_installed(prefix, expected_sha256):
                    raise RuntimeError("installed ffmpeg failed post-install version check")
            except BaseException:
                if os.path.lexists(prefix):
                    shutil.rmtree(prefix, ignore_errors=True)
                raise
    github_env = os.environ.get("GITHUB_ENV")
    if github_env:
        write_github_env(prefix, pathlib.Path(github_env))


def main(argv):
    """CLI entry point returning exit status."""
    try:
        prefix, expected_sha256 = parse_args(argv)
        install(prefix, expected_sha256)
    except (OSError, ValueError, RuntimeError, tarfile.TarError, urllib.error.URLError) as exc:
        print(f"install_ffmpeg: {exc}", file=sys.stderr)
        return 1
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        stderr = (getattr(exc, "stderr", b"") or b"").decode("utf-8", errors="replace")
        print(f"install_ffmpeg: {exc}: {stderr[-2000:]}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
