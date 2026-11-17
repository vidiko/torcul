# ADR-0002 evidence: backend measurements

Raw results: [`0002-backends.json`](0002-backends.json). Bench script:
[`tools/bench/backends.py`](../../../tools/bench/backends.py). Fixture
generator: [`tools/fixtures/track_fixtures.py`](../../../tools/fixtures/track_fixtures.py).

## How the numbers were taken

- Host: the operator workstation, 32 logical CPUs, Linux 7.3.4, measured
  2026-11-17 between 17:13 and 17:16 CET. Load average was 34 at the start and
  52 at the end (other fleet builds were running), so wall time is noisy. CPU
  time (user + system of the measured process and its children, from
  `wait4` rusage) is the figure to compare.
- Every case ran pinned to CPUs 0–7 (`taskset -c 0-7`), inside a
  memory-limited scope:

  ```bash
  systemd-run --user --scope --quiet --expand-environment=no -p MemoryHigh=8G -p MemoryMax=10G \
    python3 -I tools/bench/backends.py <work-dir> <out.json> \
    --inproc-probe <fnprobe> --inproc-remux <fnremux>
  ```

- Each row is the median of 3 runs (5 for the probe rows).
- Tool versions: FFmpeg n9.0.2, SVT-AV1 v4.2.0, x265 4.3, dav1d 1.5.4,
  GStreamer 1.28.9, mkvmerge v103.0. In-process cases use the Rust crates
  `ffmpeg-next` 9.1.0 and `ffmpeg-sys-next` 9.1.1 against the system FFmpeg.
- The codec command-line tools (SvtAv1EncApp, x265, dav1d) stand in for
  direct library use: each is its library plus a thin Y4M/IVF reader.

## Inputs

- Probe: the 11 small fixtures (320x180, 2 s) from `track_fixtures.py`.
- Remux: `f01_movie_en_de_extra.mkv` from `track_fixtures.py --large`
  (1920x1080 H.264, 120 s, 95,472,413 bytes, 4 AAC tracks, 7 SubRip tracks,
  one cover image attachment). The policy keeps input streams
  0, 1, 2, 5, 6, 7, 8, 9 and 12.
- Encode: 240 frames of FFmpeg's `testsrc2` at 1920x1080, 4:2:0 8-bit, as Y4M.

## Results

| Case | Wall (s) | CPU (s) | Peak RSS (MiB) |
| :--- | ---: | ---: | ---: |
| M1 ffprobe, one process per file (11 files) | 0.821 | 0.520 | 52.2 |
| M1 gst-discoverer, one process per file (11 files) | 5.221 | 4.073 | 269.0 |
| M1 libavformat in process (11 files, one process) | 0.122 | 0.073 | 52.2 |
| M2 ffmpeg `-c copy`, policy tracks | 0.305 | 0.187 | 65.3 |
| M2 libavformat in process, policy tracks | 0.263 | 0.139 | 61.4 |
| M2 mkvmerge, policy tracks | 0.265 | 0.156 | 76.0 |
| M2 ffmpeg `-c copy`, video + 2 audio | 0.265 | 0.154 | 59.7 |
| M2 gst-launch remux, video + 2 audio | 0.374 | 0.286 | 17.1 |
| M3 AV1 SvtAv1EncApp, preset 8, CRF 35 | 2.782 | 10.752 | 1631.7 |
| M3 AV1 ffmpeg libsvtav1, preset 8, CRF 35 | 3.357 | 11.426 | 1748.3 |
| M4 AV1 GStreamer svtav1enc, preset 8, CRF 35 | 2.739 | 11.001 | 2167.5 |
| M3 HEVC x265, preset fast, CRF 28 | 6.094 | 19.086 | 812.1 |
| M3 HEVC ffmpeg libx265, preset fast, CRF 28 | 4.617 | 18.021 | 906.0 |
| M4 HEVC GStreamer x265enc, preset fast, CRF 28 | 5.550 | 17.828 | 904.1 |
| M3 AV1 decode, dav1d CLI, 8 threads | 0.427 | 0.902 | 68.8 |
| M3 AV1 decode, ffmpeg libdav1d, 8 threads | 0.509 | 0.994 | 119.8 |

Output sizes (bytes): SVT-AV1 CLI 6,246,344; ffmpeg libsvtav1 6,246,344
(the same size, so the same encode); GStreamer svtav1enc 6,245,741 (Matroska
instead of IVF); x265 CLI 5,682,920; ffmpeg libx265 5,682,950; GStreamer
x265enc 5,621,523 (Matroska, element defaults differ slightly).

An earlier run the same afternoon (120 frames, and a remux input cut to 1.5 s
by a generator bug fixed before this run) gave the same picture: SVT-AV1
through ffmpeg +6.6 % CPU, x265 through ffmpeg +5.0 %, dav1d through ffmpeg
+29 % on a 0.3 s decode. The x265 difference changed sign between the runs,
so it is within the noise of this host; the SVT-AV1 and dav1d differences
kept their sign in both runs.

## Remux fidelity

Each remux output was read back with `mkvmerge -J`:

| Output | Tracks | Cover attachment | Tracks flagged default |
| :--- | ---: | :--- | :--- |
| Source | 12 | `cover.jpg` | audio 1 |
| ffmpeg `-c copy` | 9 | lost: written as a `V_MJPEG` video track | video 0, audio 1, subtitle 3 |
| libavformat in process | 8 | `cover.jpg`, 4511 bytes, same as source | audio 1 |
| mkvmerge | 8 | `cover.jpg` | audio 1 |
| gst-launch (video + 2 audio) | 3 | lost | all three |

Two findings follow:

1. The `ffmpeg` command-line tool adds default flags of its own when a
   track type has none. libavformat's Matroska muxer does not
   (`ffmpeg -h muxer=matroska`: `default_mode`, default `passthrough`), so an
   in-process remux keeps the source flags as they are.
2. libavformat exposes a Matroska cover image as a video stream with the
   `attached_pic` disposition, and its Matroska muxer writes such a stream
   back as a video track. The in-process remux therefore turns an
   `attached_pic` stream into an attachment stream (codec type attachment,
   picture bytes as extradata, `filename` and `mimetype` tags kept). With
   that conversion the output matches mkvmerge.

## In-process probe and remux used

The two binaries are throwaway code built only for this measurement, outside
the repository (Cargo package `fnprobe`, edition 2024, dependency
`ffmpeg-next = "9.1.0"`, release profile). Source hashes (sha256, first 12
hex): `src/main.rs` c78b7f2dd774, `src/bin/fnremux.rs` 0b381926c0e3.

`fnprobe` opens every path given to it through `ffmpeg_next::format::input`
in one process and prints index, medium, codec id and language per stream.

`fnremux <in> <out> <indices>`:

```rust
use ffmpeg_next as ffmpeg;
use ffmpeg::{Rational, codec, encoder, format};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let keep: Vec<usize> = args[3].split(',').map(|s| s.parse().expect("index")).collect();
    ffmpeg::init().expect("init");
    let mut ictx = format::input(&args[1]).expect("open input");
    let mut octx = format::output(&args[2]).expect("open output");
    let n = ictx.nb_streams() as usize;
    let mut map = vec![-1i64; n];
    let mut tbs = vec![Rational(0, 1); n];
    for (out_index, &i) in keep.iter().enumerate() {
        let ist = ictx.stream(i).expect("stream");
        map[i] = out_index as i64;
        tbs[i] = ist.time_base();
        let pic = ist.disposition().contains(format::stream::Disposition::ATTACHED_PIC);
        let mut ost = octx.add_stream(encoder::find(codec::Id::None)).expect("add");
        ost.set_parameters(ist.parameters());
        ost.set_metadata(ist.metadata().to_owned());
        ost.set_time_base(ist.time_base());
        unsafe {
            let par = ost.parameters().as_mut_ptr();
            (*par).codec_tag = 0;
            if pic {
                map[i] = -1; // picture travels as attachment data, not as a packet
                let src = &(*ist.as_ptr()).attached_pic;
                let size = src.size as usize;
                let pad = ffmpeg::ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
                let buf = ffmpeg::ffi::av_mallocz(size + pad) as *mut u8;
                std::ptr::copy_nonoverlapping(src.data, buf, size);
                (*par).codec_type = ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_ATTACHMENT;
                (*par).extradata = buf;
                (*par).extradata_size = size as i32;
            } else {
                (*ost.as_mut_ptr()).disposition = (*ist.as_ptr()).disposition;
            }
        }
    }
    octx.set_metadata(ictx.metadata().to_owned());
    octx.write_header().expect("header");
    for (stream, mut packet) in ictx.packets() {
        let i = stream.index();
        if map[i] < 0 {
            continue;
        }
        let o = map[i] as usize;
        let otb = octx.stream(o).expect("ost").time_base();
        packet.rescale_ts(tbs[i], otb);
        packet.set_position(-1);
        packet.set_stream(o);
        packet.write_interleaved(&mut octx).expect("write");
    }
    octx.write_trailer().expect("trailer");
}
```

The product remux in `torcul-media` (track-policy slice, issue #10) takes
the same steps without the panics, with a bounded packet loop and an I/O
deadline (HISS-02).

## Not measured here

- Hardware encode and decode on Intel Arc (Quick Sync Video through oneVPL,
  VA-API, Vulkan Video). The workstation has an NVIDIA card; the target cards
  sit in the cluster. This FFmpeg build has the encoders `av1_qsv`,
  `hevc_qsv`, `h264_qsv`, `av1_vaapi`, `hevc_vaapi`, `h264_vaapi`,
  `av1_vulkan`, `hevc_vulkan` and `h264_vulkan`
  (`ffmpeg -hide_banner -encoders`). The GStreamer install here has no `va`,
  `qsv` or Vulkan Video elements (`gst-inspect-1.0`), so GStreamer's hardware
  path could not be compared either. Follow-up: issue #11.
- Full-reference scoring cost: covered by ADR-0001.
