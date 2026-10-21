# SDK performance baselines

The `sdk_performance_baseline` example records runtime measurements for three reproducibly generated `GenrePreset` workloads:

| Workload | Configured characters | Locations | Factions |
|---|---:|---:|---:|
| small | 4 | 3 | 3 |
| medium | 29 | 13 | 3 |
| large | 104 | 43 | 3 |

Each workload uses seed `20261021`, advances the same number of ticks, and repeats several times. The emitted JSON contains construction time, simulation time, tick throughput, snapshot export time, checkpoint encoding time, checkpoint reconstruction time, output byte sizes, entity counts and correctness checks.

## Run locally

```sh
cargo run --locked --release -p wanxiang_sdk --example sdk_performance_baseline -- 10 3 > sdk-performance-baseline.json
python scripts/test-sdk-performance-baseline.py sdk-performance-baseline.json
```

The first argument is tick horizon (`1..=1000`); the second is repetition count (`1..=10`). Defaults are 10 ticks and 3 repeats.

## CI behaviour and interpretation

CI builds the example in release mode, verifies the report schema, confirms the workload sizes, and fails on tick/snapshot mismatch, checkpoint replay mismatch, conservation failure or excessive conservation drift. The JSON report is uploaded as the `sdk-performance-baseline` workflow artifact so results can be compared between runs.

Wall-clock figures are informational: no fixed latency/throughput gate is imposed yet. Measurements from different GitHub runners, load conditions, toolchains or profiles are not directly comparable. First establish a baseline distribution on the same runner/profile before setting thresholds; then consider robust percentiles and a tolerated regression window. This initial suite does not measure peak process memory or browser rendering cost, and its synthetic workload is not a claim about every production world shape.
