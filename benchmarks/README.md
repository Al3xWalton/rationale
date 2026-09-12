# Performance methodology

Rationale measures generated repositories rather than private application data.
The default `ava-like` scale is rounded from a local September 2026 observation
of AVA: 370 first-parent-reachable commits, 809 tracked files, and 45 Markdown,
TOML, or YAML files. No AVA names, paths, contents, history, or timings enter a
fixture or result.

| Scale | Commits | Files | Documents | Purpose |
| --- | ---: | ---: | ---: | --- |
| `small` | 25 | 100 | 10 | Fast harness calibration |
| `ava-like` | 400 | 1,000 | 50 | Version-one acceptance budget |
| `stress` | 1,024 | 5,000 | 500 | Bounded history and storage pressure |

Run the default release-mode measurement with:

```sh
make benchmark
```

Override `BENCH_SCALE`, `BENCH_SAMPLES`, or `BENCH_OUTPUT` to compare another
scale or machine. The harness records the operating system, CPU, architecture,
Rust and OCaml versions, UTC timestamp, base commit, and staged Git tree. Each
generated commit has fixed authorship, timestamp, message shape, and content.

## Measurements

- **Cold local ingestion** starts with no evidence database and includes Git
  history, repository document parsing, normalization, and first publication.
- **Incremental local sync** commits one explicit single-file change before each
  sample and measures an offline synchronization into the existing database.
- **Graph slice extraction** reads only evidence forward-reachable from the
  target commit inside one stable SQLite transaction.
- **Rust protocol codec** canonicalizes, frames, and validates one request in
  batches of 1,000 to reduce clock granularity noise.
- **OCaml kernel evaluation** runs the pure evaluator in-process after 100 warmup
  calls. **Worker round trip** measures the same request through the long-lived
  process boundary. Their median-minus-mean difference is labeled as an estimate,
  not a separately instrumented fact.
- **Warm engine**, **CLI**, and **MCP** queries reuse the published snapshot. The
  engine and MCP retain a worker; each CLI sample intentionally includes process
  and worker startup.
- **SQLite publication** inserts the full generated graph into successive atomic
  snapshots, measuring immutable-content checks and membership publication.
- **Memory stability** warms both engines, repeats engine and worker queries, and
  compares resident memory for the harness process tree before and after.

Durations use a monotonic Rust clock except the pure OCaml measurement, which
uses the platform wall clock. Distributions report minimum, nearest-rank median,
p95, and maximum. Results are machine-specific and should be compared only with
matching scale, toolchains, power conditions, and sample count.

## Enforced AVA-like budgets

- warm engine, CLI, and MCP p95 each remain below 100 ms;
- incremental offline synchronization p95 remains below one second;
- the proof request contains fewer nodes and edges than the stored snapshot;
- 200 repeated engine and worker queries add no more than 8 MiB of resident
  process-tree memory after warmup;
- repeated engine, CLI, MCP, and worker payloads remain byte-identical after
  canonicalization.

The harness writes its report before returning a failing status, so regressions
remain inspectable.
