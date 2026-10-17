# Phase 0 report

_2026-10-17 · the gate for Phase 0 in SPEC §11_

Phase 0 tested the riskiest assumptions in the design before building on them. All of it ran on the real target: a small Proxmox host, an unprivileged LXC running Docker, and the library over read-only NFS. It was reached through the real reverse proxy and TLS, under a LAN-only allow-list, with the longest book in the library (85 h). The test code is in `jail/`, `core/` and `tools/saver-spike/`.

## Summary

| Spike | Question | Result |
|---|---|---|
| (a) Offline | Can a browser store a book in IndexedDB and play it with no network? | **Pass.** Both options work on iPad Safari, Firefox for Android and Chromium, including with the network off. **Option B (service worker) is chosen; A is the fallback** (D13). |
| (b) Background playback | Does a plain `<audio>` element with Range requests keep playing with the screen off? | **Pass**, on iPad Safari and Firefox for Android. A 2 h locked session is still to be logged. |
| (c) ffmpeg | Does ffmpeg ≥ 7.1 cover the library, including xHE-AAC? | **Partial.** It probes xHE-AAC (format and chapters) but **cannot decode it**. FFmpeg 7.1.5 and 9.0.2 both fail with "not yet implemented". D10 is revised. |
| (d) Jail | Can ffmpeg be confined with Landlock, seccomp and rlimits? | **Pass.** 14 escape tests; each protection mutation-checked; real ffmpeg and ffprobe work inside it. |
| (e) Passkeys | Does `webauthn-rs` work on the real origin? | **Pass.** Registration and sign-in with user verification succeeded on Safari, and the owner reports the other devices work too. |
| (f) Data saver | Does the virtual CBR MP3 design work on real devices? | **Pass**, with two fixes found and made (below). The bitrate choice is pending. |
| (g) Sandbox in place | Do Landlock and seccomp work inside Docker inside an unprivileged LXC? | **Pass**, once Landlock was added to the host kernel's `lsm=` list (D24). |

## (f) Data saver in detail

- **Exactness:** the 32 kb/s virtual file for a 2 h book was 28,800,192 bytes, exactly as advertised. It decoded with no errors across all 121 joins.
- **Seek latency on the target:**
  - first audio from an uncached position in 0.72–0.80 s on the 85 h book;
  - iPad Safari seeks completed in a median of 240 ms (worst 724 ms).
- **Segment cost:** about 0.7 s per 60 s segment on the target. Of that, about 0.5 s is ffmpeg opening the 85 h file and reading its large index; the encode itself is about 0.2 s.
- **Finding 1: browsers read far ahead.** Safari read about 72 minutes ahead on an open-ended range. **Fix:** each response is capped at 10 segments (a short 206), and the media element simply asks again. Tested through the proxy.
- **Finding 2: scrub storms.** Firefox for Android's native scrubber fires about 30 seeks a second, each starting an encode. After about 30 s Firefox gave up and jumped to the end. **Fixes:**
  - the app's own scrubber seeks only on release;
  - **latest seek wins** on the server: a queued encode superseded by a newer request is dropped.

  In a storm of 15 seeks, 13 were dropped and the final seek answered in 288 ms (2.9 s before). The owner confirmed the custom scrubber works on Android.
- **Finding 3: data-saver downloads are slow per segment.** At about 0.34 MB/s on the target, a whole 85 h book takes about an hour. **Fix (Phase 1):** downloads and prefetch encode many consecutive segments per ffmpeg run, so the file is opened once.
- **Bitrate:** 32, 40 and 48 kb/s were all deployed. The owner can hear the difference between them; the default is still to be chosen (§12).

## (a) Offline in detail

- **iPad Safari** (Home Screen app): a 64 MiB piece of the data-saver stream (4.6 h of audio) was downloaded in 28 pieces. The Blob for option A was built in 10–21 ms. Both options played and seeked from 0 to 8400 s, before and after the device went offline.
- **Headless Chromium:** both options played and seeked to 50, 75 and 90 % with the server stopped.
- **Findings:**
  1. Downloads run only while the page is open. Firefox aborted the fetch when the page was hidden, and iOS has no background-fetch API. Downloads resume piece by piece with an ETag check, and the UI will say to keep ABShare open while downloading.
  2. The service worker must not be re-registered while offline, because that fetches the script. This was found and fixed in testing.
  3. `navigator.storage.persist()` returns false in a Safari tab and true in Firefox for Android. Persistent storage on iOS needs the Home Screen app, as the spec already says.
- **Choice: option B (service worker).** It answers Range requests from storage without building one huge object, plays partial downloads naturally, and keeps memory flat for 1–2.5 GB books. Option A stays as the fallback where service workers are unavailable (for example some private-browsing modes).

## (c) xHE-AAC in detail

Store-bought audiobooks may come as xHE-AAC (USAC, AAC object type 42). FFmpeg recognises the format, reads the chapters and reports `profile=xHE-AAC`, but decoding fails throughout with "Not yet implemented in FFmpeg" and corrupt-data errors. **The process still exits 0.**

**Consequences:**
- The scanner runs a short trial decode on every new file and treats **any decoder error** as unplayable. The exit code alone is not enough.
- An xHE-AAC file with an AAC-LC alternate in the same folder (a common result of re-downloading) is handled by the alternates rule (§4.3).
- An xHE-AAC file with no alternate is listed to admins as "xHE-AAC: not decodable, re-download as AAC".
- The only decoder that handles these files, Fraunhofer's libfdk-aac, has a licence incompatible with the GPL, so it is not bundled.
- Where an xHE-AAC file has no AAC-LC alternate, the book has to be re-downloaded as AAC.

## Decisions taken

- **D10** revised: ffmpeg ≥ 7.1 is still required, but xHE-AAC is unsupported, and the scanner detects it by trial decode.
- **D13** resolved: option B, with A as fallback.
- **D23** amended: the read-ahead cap, latest-seek-wins, and multi-segment encoding for downloads.
- **D25** added: `webauthn-rs` with vendored OpenSSL.

## Still open

- A logged 2 h locked-screen session (data saver and original).
- The data-saver bitrate choice (32, 40 or 48 kb/s).
- An offline test of a complete 1+ GB download on iOS. This is expected to work with option B, which never builds one large object.
