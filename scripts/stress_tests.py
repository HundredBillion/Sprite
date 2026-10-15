#!/usr/bin/env python3
"""Repeat a Cargo test binary under CPU load, retaining every run's result."""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "app": ("sprite-app", ["--lib"], "sprite_app"),
    "client": ("sprite-app", ["--test", "client"], "client"),
    "graphics_policy": ("sprite-term", ["--test", "graphics_policy"], "graphics_policy"),
    "graphics_tmux": ("sprite-term", ["--test", "graphics_tmux"], "graphics_tmux"),
}


def run_once(command, cwd, log, timeout, env):
    with tempfile.TemporaryDirectory(prefix="ss-", dir="/tmp") as scratch:
        return run_isolated(command, cwd, log, timeout, {**env, "TMUX_TMPDIR": scratch}, Path(scratch))


def run_isolated(command, cwd, log, timeout, env, scratch):
    with log.open("w") as output:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdout=output,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            return child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            return 124
        finally:
            # Descendants may still hold sockets or pipes after their test exits.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
            # tmux daemonizes, so it escapes the test process group.
            for socket in scratch.glob("tmux-*/*"):
                if socket.is_socket():
                    subprocess.run(["tmux", "-S", str(socket), "kill-server"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS)
    parser.add_argument("--runs", type=int, default=100)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--no-load", action="store_true")
    args = parser.parse_args()
    if args.runs < 1 or args.timeout <= 0:
        parser.error("runs and timeout must be positive")
    if args.target == "graphics_tmux" and not shutil.which("tmux"):
        parser.error("tmux is required; skipped graphics tests are not stress evidence")
    def interrupted(signum, frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    args.output.mkdir(parents=True, exist_ok=False)
    package, selector, name = TARGETS[args.target]
    build = subprocess.run(
        ["cargo", "test", "-p", package, *selector, "--locked", "--offline",
         "--no-run", "--message-format=json"], cwd=ROOT, text=True,
        stdout=subprocess.PIPE, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines()]
    executables = [a["executable"] for a in artifacts
                   if a.get("reason") == "compiler-artifact"
                   and a.get("executable") and a["target"]["name"] == name
                   and a["profile"]["test"]]
    if len(executables) != 1:
        raise RuntimeError(f"expected one {name} test executable: {executables}")
    env = {**os.environ, "SPRITE_TEST_WATCHDOG_SECS": os.environ.get("SPRITE_TEST_WATCHDOG_SECS", "20")}
    report = {"platform": platform.platform(), "machine": platform.machine(),
              "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
              "target": args.target, "command": executables,
              "load_processes": 0 if args.no_load else os.cpu_count() or 1,
              "watchdog_seconds": env["SPRITE_TEST_WATCHDOG_SECS"], "runs": []}
    workers = []
    try:
        for _ in range(report["load_processes"]):
            workers.append(subprocess.Popen(["yes"], stdout=subprocess.DEVNULL))
        for iteration in range(1, args.runs + 1):
            started = time.monotonic()
            code = run_once(executables, ROOT / "crates" / package,
                            args.output / f"{iteration:04}.log", args.timeout, env)
            report["runs"].append({"run": iteration, "status": code,
                                   "seconds": round(time.monotonic() - started, 3)})
            temporary = args.output / "results.json.tmp"
            temporary.write_text(json.dumps(report, indent=2) + "\n")
            temporary.replace(args.output / "results.json")
            if code or iteration % 10 == 0:
                print(f"{args.target}: {iteration}/{args.runs}, exit={code}", flush=True)
    finally:
        for worker in workers:
            worker.terminate()
        for worker in workers:
            worker.wait()
    failures = sum(run["status"] != 0 for run in report["runs"])
    print(f"{args.target}: {args.runs - failures}/{args.runs} passed", flush=True)
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
