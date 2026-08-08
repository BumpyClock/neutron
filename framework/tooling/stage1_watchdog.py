#!/usr/bin/env python3
"""Run one CI command under a bounded external watchdog."""

from __future__ import annotations

import argparse
import datetime as dt
import os
from pathlib import Path
import shlex
import subprocess
import sys
from typing import TextIO

import stage1_process


def command_text(command: list[str]) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(command)
    return shlex.join(command)


def log_line(log: TextIO, message: str) -> None:
    timestamp = dt.datetime.now(dt.timezone.utc).isoformat()
    log.write(f"{timestamp} {message}\n")
    log.flush()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a command with a wall-clock timeout and preserve its output.",
        allow_abbrev=False,
    )
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument(
        "--cleanup-seconds",
        type=float,
        default=stage1_process.DEFAULT_CLEANUP_SECONDS,
        help="Hard post-timeout cleanup allowance; defaults to 5 seconds.",
    )
    parser.add_argument("--stdout", type=Path, required=True)
    parser.add_argument("--stderr", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--stdin", type=Path)
    parser.add_argument(
        "--expected-exit-code",
        action="append",
        type=int,
        dest="expected_exit_codes",
        help="Exit code considered successful; defaults to 0.",
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be greater than zero")
    if args.cleanup_seconds <= 0:
        parser.error("--cleanup-seconds must be greater than zero")
    if args.stdout == args.stderr:
        parser.error("--stdout and --stderr must name different files")

    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")

    if args.expected_exit_codes is None:
        args.expected_exit_codes = [0]
    return args


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def main() -> int:
    args = parse_args()
    for path in (args.stdout, args.stderr, args.log):
        ensure_parent(path)

    args.stdout.write_bytes(b"")
    args.stderr.write_bytes(b"")
    expected_exit_codes = set(args.expected_exit_codes)

    with args.log.open("w", encoding="utf-8") as log:
        log_line(log, f"command={command_text(args.command)}")
        log_line(log, f"timeout_seconds={args.timeout_seconds:g}")
        log_line(log, f"cleanup_seconds={args.cleanup_seconds:g}")
        log_line(log, f"expected_exit_codes={sorted(expected_exit_codes)}")

        stdin = None
        try:
            if args.stdin is not None:
                stdin = args.stdin.open("rb")
                log_line(log, f"stdin={args.stdin}")

            try:
                result = stage1_process.run_capture(
                    args.command,
                    timeout_seconds=args.timeout_seconds,
                    cleanup_seconds=args.cleanup_seconds,
                    stdin=stdin if stdin is not None else subprocess.DEVNULL,
                )
            except OSError as error:
                message = f"could not start command: {error}\n"
                args.stderr.write_text(message, encoding="utf-8")
                log_line(log, message.rstrip())
                return 127

            args.stdout.write_bytes(result.stdout)
            args.stderr.write_bytes(result.stderr)
            if result.timed_out:
                log_line(
                    log,
                    f"timeout after {result.elapsed_seconds:.3f} seconds; process-tree termination requested",
                )
                if result.cleanup_timed_out:
                    log_line(
                        log,
                        "cleanup allowance expired before process termination and output draining were confirmed",
                    )
                else:
                    log_line(log, "process termination and output draining confirmed")
                log_line(
                    log,
                    f"stdout_bytes={len(result.stdout)} stderr_bytes={len(result.stderr)}",
                )
                return 124

            log_line(
                log,
                f"exit_code={result.returncode} elapsed_seconds={result.elapsed_seconds:.3f}",
            )
            log_line(
                log,
                f"stdout_bytes={len(result.stdout)} stderr_bytes={len(result.stderr)}",
            )
            if result.cleanup_timed_out:
                log_line(
                    log,
                    "cleanup allowance expired before process termination and output draining were confirmed",
                )
                return 1
            if result.returncode not in expected_exit_codes:
                log_line(log, "command exited with an unexpected status")
                return 1
            return 0
        finally:
            if stdin is not None:
                stdin.close()


if __name__ == "__main__":
    sys.exit(main())
