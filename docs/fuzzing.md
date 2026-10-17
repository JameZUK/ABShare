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

CI runs every target for 60 s on each push (D21).
