# Local application logging

## Outcome

Developers can locate and follow Sprite application diagnostics during a local reproduction, whether the window was launched from a shell or the desktop. Diagnostics support investigation, not remote telemetry or a claim of improved performance. This work implements issue #117 independently of the stopped AI-native refactor and pane-control work.

## Design

Initialize logging only after argument parsing selects the graphical window invocation, before configuration is loaded. CLI commands retain their existing stdout, stderr, and exit behavior, including machine-readable replies and Surface streams. A small internal logging module configures established Rust logging components; it is not an external service. Evaluate `tracing` and its subscriber and writer components against these requirements, and add custom code only where needed to satisfy them.

Events have timestamps, severity, subsystem, process identity, and relevant available window, pane, session, or request identifiers. Default to warnings and errors; provide an environment-controlled level and subsystem filter for opt-in lifecycle diagnostics and timings. Preserve existing user-facing configuration complaints. Instrument startup/configuration, pane/session creation and cleanup, IPC and Surface failures, and shutdown. Use monotonic elapsed timings with units and correlation context for startup and selected request handling. Avoid per-character and per-frame events.

Write to discoverable per-user files under `~/Library/Logs/Sprite/` on macOS and the XDG state directory on Linux. A bounded, nonblocking queue feeds a background writer. Rotate by size and retain a fixed documented limit, including a safe strategy for simultaneous processes that does not delete an active process's logs. Define concrete limits in the implementation and documentation. A saturated queue drops diagnostics and reports a count where feasible. An unavailable, slow, or full destination must not prevent normal terminal operation. Shutdown flushing is best effort and must not assume abrupt crashes can flush. Record limited panic context on a best-effort basis while preserving the existing panic hook.

Never record raw terminal input or output, credentials, tokens, or full environment values by default. Diagnostics remain outside pane content and pane-observation replies. Document that explicitly displaying a log in a pane makes it visible to clients authorized for that pane, and filesystem-capable agents may separately read log files.

## Acceptance and evidence

- Document locations, active-file discovery, live following and filtering, configuration, retention, destination failures, dropped events, panic/crash limits, and privacy boundaries.
- Verify level and subsystem filtering, unchanged CLI/protocol output, file creation and retention including concurrent processes, bounded queue saturation, and unwritable or full destination handling.
- Measure overhead with detailed logging disabled and enabled on representative idle and high-output workloads. Record workload, environment, methodology, results, and native-platform limitations. Distinguish elapsed latency from CPU time and direct developers to profiling or existing benchmarks when appropriate.
- Run relevant automated tests and report any platform-specific verification that could not be performed. No remote collector, dashboard, general metrics platform, or new agent architecture is in scope.
