# BENCHMARKS.md — Results against the performance goals

Measured on **2026-10-28** (Phase 4).

- **Machine:** Intel Core i7-1355U (10 cores / 12 threads), 38 GB RAM, NVMe (Micron),
  CachyOS (kernel 7.3.0), Rust 1.99.1, `release` profile.
- **File:** generated with `lljson-cli generate` (seed 1): array of 83,334 records,
  **1,000,009 nodes**, 20.7 MiB indented with 2 spaces (13.7 MiB minified). Includes
  escapes, unicode, decimal numbers, `null` and 3 levels of nesting.
  The "10 changes" variant is generated with `--modify 10`.
- **How to reproduce:**
  ```sh
  cargo bench -p lljson-core                  # criterion: parse, hash, diff, format
  cargo bench -p lljson-core --bench memory   # RSS with both documents loaded
  ```

## Summary against the SPEC.md goals

| Metric | Goal | Result | Status |
|---|---|---|---|
| Parsing (1M nodes) | < 1 s | **53 ms** (strict), 57 ms (tolerant) | ✅ ~19× margin |
| Diff of two nearly identical files | < 500 ms | **8.2 ms** diff only; **72 ms** end to end (read + parse both + diff) | ✅ ~7× margin end to end |
| Memory with both files loaded | < 500 MB | **158 MiB** RSS (2 × 1M nodes + diff) | ✅ ~3× margin |
| App startup | < 300 ms | **~210 ms** until the 1,250,012 rows are in the view (first frame at ~70–100 ms) | ✅ |
| Scroll at 60 fps | smooth | **59–59.4 fps** average, p99 ≤ 33 ms (60 Hz monitor) | ✅ |
| App memory with both files | < 500 MB | **~240–270 MiB** RSS | ✅ |

## Detail (criterion, median)

### Parsing — `benches/parse.rs`
| Option | Time | Throughput |
|---|---|---|
| lljson strict | 52.9 ms | 391 MiB/s |
| lljson tolerant | 57.3 ms | 361 MiB/s |
| serde_json → `Value` | 199.9 ms | 103 MiB/s |
| simd-json → `BorrowedValue` | 73.1 ms | 283 MiB/s |
| simd-json → tape | 26.9 ms | 768 MiB/s |

Includes building the arena and hashing all subtrees. The Phase 1 decision
(custom parser) stands: only the simd-json tape is faster, and it provides no arena,
tolerant mode or ranges over the original text.

### Hash — `benches/hash.rs`
| Case | Time |
|---|---|
| Recompute all hashes over the arena (estimate) | 28.1 ms |
| Compare two documents by the root hash | < 1 ns |

The hash is computed inside the parser and cannot be isolated; the estimate repeats the same
xxh3 work over the already-built arena (without the canonical form of numbers), so
it is an approximate bound on its weight within the ~53 ms of parsing.

### Diff — `benches/diff.rs`
| Case | Changes found | Time |
|---|---|---|
| Identical | 0 | 0.33 ms |
| 10 changes (by index) | 10 modified | 8.2 ms |
| 10 mixed changes, arrays by key `id` | 1 added, 2 removed, 7 modified (2 type changes) | 15.8 ms |
| 10 changes ignoring `score` and `$[*].address.zip` | 10 modified | 9.6 ms |
| 1 record inserted at the start, by index | 649,918 (everything shifts) | 84.1 ms |
| 1 record inserted at the start, by key `id` | 1 added | 13.6 ms |

The worst reasonable case (an insertion that shifts all indices) is still 6× under the goal.
With `--array-key id` the same case is reduced to the single real change.

### Formatting — `benches/format.rs` (minified input)
| Operation | Time | Throughput |
|---|---|---|
| Format (2 spaces) | 30.6 ms | 448 MiB/s |
| Minify | 22.1 ms | 618 MiB/s |
| Sort keys | 54.2 ms | 252 MiB/s |

### Memory — `benches/memory.rs`
| Stage | RSS |
|---|---|
| Initial | 24.3 MiB |
| After opening (mmap) and parsing both documents | 157.7 MiB (+133.4) |
| After the diff | 157.7 MiB (+0.0) |

Theoretical: 41.3 MiB of text (mmap) + 91.6 MiB of arena (`Node` = 48 bytes × 2,000,018 nodes).
The diff does not raise the RSS because it reuses memory the parser already freed. The UI
memory (flat row list and visible widgets) is still to be measured in phases 6 and 7.

### End to end with the CLI (10 runs, median)
| Command | Time | Peak RSS |
|---|---|---|
| `diff -q a.json b.json` (10 changes) | 72 ms | ~150 MiB |
| `diff -q --array-key id a.json b.json` | 83 ms | ~153 MiB |
| `format a.json` | 79 ms | ~153 MiB |
| `minify a.json` | 71 ms | ~153 MiB |

Includes process startup, reading with mmap, parsing (both documents in
parallel in `diff`) and writing the output to `/dev/null`.

## GTK app with 1 million nodes (Phase 6)

Measured on **2026-10-28**, `release` profile with the `devtools` feature, Wayland, 60 Hz monitor,
same machine as above. Files: `generate` (seed 1) and `generate --modify 10 --type-change 5`,
21.7 MB each, 1,250,012 aligned rows, content height 23.75 M px.

```sh
lljson-cli generate > a.json
lljson-cli generate --modify 10 --type-change 5 > b.json
LLJSON_TRACE=1 LLJSON_SCROLL_TEST=10 cargo run --release -p lljson --features devtools -- a.json b.json
```

| Metric | Result (3 runs) |
|---|---|
| First frame | 71–101 ms |
| Documents loaded (each, in parallel) | 52–62 ms |
| Rows ready in the view | 208–224 ms from startup |
| Continuous scroll (5 s, 3000 px/s) | 58.4–59.4 fps; p50/p95 16.7 ms; 2–3 frames > 25 ms out of ~295; max 50–100 ms |
| Random jumps (5 s) | 59.0–59.2 fps; p95 16.7 ms; p99 31–33 ms; 4–5 frames > 25 ms; max 33 ms |
| Final RSS | 242–273 MiB (VmHWM the same) |

The isolated slow frames (≈1%) are still to be investigated; the p95 is at the monitor's
refresh interval. Known limitation: **expanding the root takes ~54 ms on the UI thread**
(`benches/rows.rs`), above the 16 ms of a frame.

### Navigating between differences — `benches/rows.rs` (Phase 7)
| Case | Time |
|---|---|
| `find_change`, worst case: scan all 1,250,012 rows without finding any | 3.0 ms |

Runs on the UI thread (goal: < 16 ms per frame).

### Hiding equal sections (Phase 7)
With the two 1 M-node files (`--modify 10 --type-change 5`), turning the switch on
takes the rows from 1,250,012 to **83,456**. The rebuild (diff + rows) runs on a background
thread and takes ~190 ms until the view updates; the UI thread only receives the result.

### Search — `Query::find` (Phase 10)
12 threads, release profile, `stress_5000000.json` (19,282,827 nodes), search over keys and
values on background threads (the UI thread only receives the result):

| Query | Mode / scope | Hits | Time |
|---|---|---|---|
| `key_0000138` | Normal / both | 1 | 108 ms |
| `comilla` | Normal / values | 79,323 | 91 ms |
| `ZZZ_no_existe` | Normal / both | 0 | 102 ms |
| `true` | Normal / values | 1,071,607 | 92 ms |
| `Comilla` | Exact / values | 0 | 94 ms |
| `^s_a8[0-9a-f]+$` | Regex / values | 0 | 169 ms |

Parsing the same file takes 1.06 s. `Rows::reveal` (expanding the ancestors of a result) runs on
the UI thread, like a manual expand; it scans the visible rows once per collapsed level.
