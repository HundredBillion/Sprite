# Local logging measurements

Measured 2026-10-26 on macOS 26.5.2, Apple M5 Pro, 18 logical CPUs, 64 GiB RAM. The tested `sprite` was a release build made with Homebrew Rust 1.98.1. These are local measurements, not performance budgets or an idle-machine certification. They were taken before the final retention cleanup hardening and lint-only edits; the measured per-event logging path was unchanged.

From the repository root, build with `cargo build --release --locked -p sprite-app --bin sprite`, then run `python3 scripts/measure-logging.py`. The script starts a real Sprite window for each run. The idle command is `sleep 20`; the output command sends up to 10 MiB through the pane using `yes 0123456789 | head -c 10485760`, then sleeps. After four seconds it samples the Sprite process using `ps -o time=,rss=`, terminates the process, and records each result in `/tmp/sprite-log-measure.json`. `TIME` is cumulative CPU time for the Sprite process, excluding the shell and other children; RSS is a single resident-memory sample in KiB. The four-second window does not assert that every byte was rendered. The test uses Sprite's normal file sink in both modes and alternates `SPRITE_LOG=sprite_app=warn` with `SPRITE_LOG=sprite_app=debug`, three samples per combination. Detailed diagnostics do not emit per-character or per-frame events.

| Workload | Filter | Median process CPU time | Median RSS (KiB) |
| --- | --- | ---: | ---: |
| Idle | warn | 0.21 s | 84,992 |
| Idle | debug | 0.21 s | 84,864 |
| Output | warn | 1.36 s | 97,104 |
| Output | debug | 1.39 s | 97,088 |

The [raw samples](local-logging-macos.json) include each process's sampled CPU time, RSS, elapsed time and exit state. All twelve processes were still running at sampling time and produced no stderr. Three short, fixed-order samples are insufficient to establish a performance improvement or a reliable overhead percentage. Normal logging is active in both modes, so these results do not compare against a build without logging. No native Linux performance measurement was available in this run. Use a profiler and longer controlled benchmarks for CPU, rendering throughput or memory investigations.
