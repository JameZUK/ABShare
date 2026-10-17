# Fuzzing results

Coverage-guided fuzzing with `cargo-fuzz`/libFuzzer (SPEC D21). The targets are in `fuzz/fuzz_targets/`.

## Phase 1 gate: one hour per target (2026-10-17)

Each target ran for 3,600 s on one core, from the seed corpora in `fuzz/corpus/`. **There were no crashes, panics, timeouts or out-of-memory failures, and `fuzz/artifacts/` stayed empty.**

| Target | What it exercises | Runs in 1 h |
|---|---|---:|
| `saver_segment` | Data-saver segment and range maths | 1,021,347,601 |
| `range_header` | HTTP `Range` parsing | 775,152,244 |
| `progress` | Progress merge rules | 212,540,602 |
| `ffprobe_json` | ffprobe JSON parsing and probing | 88,727,035 |
| `chapters` | Chapter building and cleaning | 26,098,846 |
| `episode_title` | Episode and show title rules | 24,632,480 |
| `metadata` | Tag normalisation into book metadata | 16,362,695 |
| `html_text` | HTML-to-text for descriptions | 16,069,620 |
| `assemble` | Grouping files into books | 7,235,686 |

Before this campaign, short runs had found one bug: a lone `<` or `>` survived description cleaning. The fix routes any description containing `<`, `>` or `&` through `html_to_text`, and a regression test covers it.

## Targets added since

Two targets were added after the Phase 1 campaign, in the Phase 6 review: `metadata_override` (admin corrections to book details) and `genre` (genre tags onto shelves, which also exercises text cleaning). They run in CI with the others; no one-hour campaign has been recorded for them yet.

The `metadata` target uses a dictionary (`fuzz/dicts/metadata.dict`) of characters whose lower-case form changes length, the class of input behind review findings C1 and C2.

## Running a campaign

```sh
cargo +nightly fuzz run <target> -- -max_total_time=3600
```

CI runs all 11 targets for 60 s on each push (D21).
