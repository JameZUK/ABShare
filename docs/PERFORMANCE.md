# Performance

What ABShare costs to run, and how quickly it answers, from measurements. Each number says where and how it was taken. Numbers from the Phase 0 prototype are marked as such; they were taken before the real server existed, so treat them as indicative.

## Memory and CPU at rest

| What | Measured |
|---|---|
| `abshare serve`, idle, catalogue loaded | 6.0 MiB resident |
| `abshare scan --every 3600`, between scans | 3.9 MiB resident |

Measured with `docker stats` on 2026-10-17, shortly after a restart, in the release image on a 2-core container (Intel i5-9500T) serving a real library. CPU at rest was 0 %. Memory grows with the size of the catalogue (it is held in memory, §6.2 of [`ARCHITECTURE.md`](ARCHITECTURE.md)) and while ffmpeg jobs run; the Compose example allows 2 GiB for the server and 1 GiB for the scanner, mostly for ffmpeg's address-space reservation.

## Streaming

- **Original quality** costs almost nothing: the server reads a slice of the file and sends it. A multi-part book or one in a format browsers can't play needs a playable copy first, built once in the background and then served the same way.
- **Data saver** transcodes on demand, in 60-second segments, and caches them.

| Measurement | Result | Where |
|---|---|---|
| One 60 s data-saver segment from the middle of a 2 h AAC file | 0.19 s on one core (about 325× real time) | Phase 0 prototype, scratch test |
| First audio after an uncached seek, 85 h book | 0.72–0.80 s | Phase 0(f) spike on the same 2-core host, library over read-only NFS |
| Seek latency on iPad Safari | median 240 ms, worst 724 ms | Phase 0 device test |
| Fifteen rapid seeks (a scrubbing storm) | 13 dropped by *latest seek wins*; the final seek answered in 288 ms (2.9 s before the rule) | Phase 0 storm test |

On a long book most of a segment's cost is ffmpeg reading the file's index, which is why downloads (`dl=1`) encode up to 10 segments per ffmpeg run, and why each response is capped at 10 segments: a browser reading far ahead (Safari read about 72 minutes ahead in testing) can't make the server transcode the whole book at once.

## Data-saver sizes

The data-saver stream is constant bitrate, so its size is exact:

| Bitrate | Per hour | A 10-hour book |
|---|---:|---:|
| 48 kb/s (default) | 21.6 MB | 216 MB |
| 40 kb/s | 18.0 MB | 180 MB |
| 32 kb/s | 14.4 MB | 144 MB |

The 10 GiB default cache holds about 500 hours of data-saver audio at 48 kb/s, less whatever playable copies take.

## Offline playback

| Measurement | Result | Where |
|---|---|---|
| Composing 13 stored pieces into one Blob (the fallback path) | 3 ms | Phase 0, Chromium |
| The same with 28 pieces | 10–21 ms | Phase 0, iPad Safari |

The app uses the service-worker path, which reads only the pieces a Range request needs, so start-up doesn't depend on the size of the book.

## Scanning

A scan reads only directory metadata (sizes and modification times) unless a file is new or changed, so a rescan of an unchanged library is quick and doesn't wake cold storage. New files are probed and trial-decoded in the jail, `[ffmpeg] jobs × 2` at a time; the first scan of a large library is dominated by those probes and by cover extraction.

## Measuring

- Memory: `docker stats --no-stream`, or `ps -o rss= -C abshare`.
- Seek latency: the browser's network panel, from the Range request to its first byte.
- Segment cost: run a data-saver Range request against an uncached book and time it.
- Scan time: the admin Library page shows how long the last scan took.
