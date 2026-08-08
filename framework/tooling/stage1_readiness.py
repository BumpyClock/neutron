#!/usr/bin/env python3
"""Wait for a command to prove a CI service is ready."""

from __future__ import annotations

import argparse
import datetime as dt
from pathlib import Path
import shlex
import os
import subprocess
import sys
import time
from typing import TextIO

import stage1_process


def log_line(log: TextIO, message: str) -> None:
    timestamp = dt.datetime.now(dt.timezone.utc).isoformat()
    log.write(f"{timestamp} {message}\n")
    log.flush()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Poll a readiness command until it succeeds or times out.",
        allow_abbrev=False,
    )
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--probe-timeout-seconds", type=float, default=2)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be greater than zero")
    if args.probe_timeout_seconds <= 0:
        parser.error("--probe-timeout-seconds must be greater than zero")
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("a readiness command is required after --")
    return args


def main() -> int:
    args = parse_args()
    args.log.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.timeout_seconds

    with args.log.open("a", encoding="utf-8") as log:
        command = (
            subprocess.list2cmdline(args.command) if os.name == "nt" else shlex.join(args.command)
        )
        log_line(log, f"readiness_command={command}")
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                log_line(log, "readiness timeout expired")
                return 124

            try:
                result = stage1_process.run_capture(
                    args.command,
                    timeout_seconds=min(args.probe_timeout_seconds, remaining),
                )
            except OSError as error:
                log_line(log, f"readiness command could not start: {error}")
                return 127
            log.write(result.stdout.decode("utf-8", errors="replace"))
            log.write(result.stderr.decode("utf-8", errors="replace"))
            log.flush()

            if result.cleanup_timed_out:
                log_line(log, "readiness command cleanup exceeded its hard allowance")
                return 124
            if not result.timed_out and result.returncode == 0:
                log_line(log, "readiness command succeeded")
                return 0

            # This delay only spaces repeated failed probes; readiness is never
            # inferred from elapsed time.
            time.sleep(min(0.1, max(0.0, deadline - time.monotonic())))


if __name__ == "__main__":
    sys.exit(main())
