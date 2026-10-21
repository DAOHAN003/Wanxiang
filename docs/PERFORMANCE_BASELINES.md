# SDK performance baselines

The `sdk_performance_baseline` example records runtime measurements for three reproducibly generated `GenrePreset` workloads:

| Workload | Configured characters | Locations | Factions |
|---|---:|---:|---:|
| small | 4 | 3 | 3 |
| medium | 29 | 13 | 3 |
| large | 104 | 43 | 3 |

Each workload uses seed `20261021`, advances the same number of ticks, and repeats several times. The emitted JSON contains release build time, per-workload construction time, simulation time, tick throughput, snapshot export time, checkpoint encoding time, checkpoint reconstruction time, output byte sizes, entity counts and correctness checks. The CI wrapper also measures the benchmark process' peak resident set size (RSS) and CPU time with GNU `/usr/bin/time -v`; compilation memory is intentionally excluded.

## Run locally

```sh
python scripts/run-sdk-performance-baseline.py --ticks 10 --repeats 3 --output sdk-performance-baseline.json
python scripts/test-sdk-performance-baseline.py sdk-performance-baseline.json
```

The runner accepts a tick horizon (`1..=1000`), repetition count (`1..=10`) and output path. Defaults are 10 ticks and 3 repeats. Peak RSS currently uses Linux GNU `/usr/bin/time`; other operating systems are not supported by this memory-measurement wrapper.

## CI behaviour and interpretation

CI builds the example in release mode, verifies the report schema and workload sizes, and fails on tick/snapshot mismatch, checkpoint replay mismatch, a failed four-pool conservation invariant, or non-finite diagnostic drift. Drift is reported separately from the conserved flag and is not assigned a hard threshold in this first baseline. The JSON report is uploaded as the `sdk-performance-baseline` workflow artifact so results can be compared between runs.

Wall-clock figures are informational: no fixed latency/throughput gate is imposed yet. Measurements from different GitHub runners, load conditions, toolchains or profiles are not directly comparable. First establish a baseline distribution on the same runner/profile before setting thresholds; then consider robust percentiles and a tolerated regression window. Browser rendering cost is measured separately by the Playwright renderer benchmark and uploaded in the `world-lab-browser-evidence` artifact. Peak RSS records the single benchmark process' high-water mark, not total host memory; the synthetic workload is not a claim about every production world shape.
