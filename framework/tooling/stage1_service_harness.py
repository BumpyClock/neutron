#!/usr/bin/env python3
"""Run one Stage 1 payload while retaining and cleaning its native service tree."""

from __future__ import annotations

import argparse
import datetime as dt
from pathlib import Path
import shlex
import sys
import time
from typing import TextIO

import stage1_process


def log_line(log: TextIO, message: str) -> None:
    timestamp = dt.datetime.now(dt.timezone.utc).isoformat()
    log.write(f"{timestamp} {message}\n")
    log.flush()


def command_text(command: list[str]) -> str:
    return shlex.join(command)


def prepare_output(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"")


def write_output(path: Path, output: bytes) -> None:
    path.write_bytes(output)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--cleanup-seconds", type=float, default=5.0)
    parser.add_argument("--service-executable", required=True)
    parser.add_argument("--service-argument", action="append", default=[])
    parser.add_argument("--service-stdout", type=Path, required=True)
    parser.add_argument("--service-stderr", type=Path, required=True)
    parser.add_argument("--payload-stdout", type=Path, required=True)
    parser.add_argument("--payload-stderr", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("payload", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be greater than zero")
    if args.cleanup_seconds <= 0:
        parser.error("--cleanup-seconds must be greater than zero")
    if args.payload[:1] == ["--"]:
        args.payload = args.payload[1:]
    if not args.payload:
        parser.error("payload command is required after --")
    return args


def monitor(
    service: stage1_process.ManagedProcess,
    payload: stage1_process.ManagedProcess,
    deadline: float,
) -> tuple[str, int | None]:
    while True:
        service_status = service.poll()
        if service_status is not None:
            return "service_exited", service_status
        payload_status = payload.poll()
        if payload_status is not None:
            return "payload_exited", payload_status
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return "timed_out", None
        time.sleep(min(0.01, remaining))


def main() -> int:
    if sys.platform != "linux":
        print("stage1_service_harness.py requires Linux", file=sys.stderr)
        return 2

    args = parse_args()
    for path in (
        args.service_stdout,
        args.service_stderr,
        args.payload_stdout,
        args.payload_stderr,
        args.log,
    ):
        prepare_output(path)

    service_command = [args.service_executable, *args.service_argument]
    payload_command = list(args.payload)
    started = time.monotonic()
    execution_deadline = started + args.timeout_seconds
    service = None
    payload = None
    service_threads = ()
    payload_threads = ()
    service_stdout = bytearray()
    service_stderr = bytearray()
    payload_stdout = bytearray()
    payload_stderr = bytearray()
    outcome = "launch_failed"
    observed_status = None
    cleanup_succeeded = False

    with args.log.open("w", encoding="utf-8") as log:
        log_line(log, f"service_command={command_text(service_command)}")
        log_line(log, f"payload_command={command_text(payload_command)}")
        log_line(log, f"timeout_seconds={args.timeout_seconds:g}")
        log_line(log, f"cleanup_seconds={args.cleanup_seconds:g}")
        try:
            service = stage1_process.start_process(service_command)
            try:
                service_stdout, service_stderr, service_threads = (
                    stage1_process.start_output_pumps(
                        service,
                        failure_policy=stage1_process.PumpStartFailurePolicy.DEFER_TO_OWNER,
                    )
                )
            except stage1_process.OutputPumpStartError as error:
                service_stdout = error.stdout
                service_stderr = error.stderr
                service_threads = error.threads
                raise

            if time.monotonic() >= execution_deadline:
                outcome = "timed_out"
            else:
                payload = stage1_process.start_process(payload_command)
                try:
                    payload_stdout, payload_stderr, payload_threads = (
                        stage1_process.start_output_pumps(
                            payload,
                            failure_policy=stage1_process.PumpStartFailurePolicy.DEFER_TO_OWNER,
                        )
                    )
                except stage1_process.OutputPumpStartError as error:
                    payload_stdout = error.stdout
                    payload_stderr = error.stderr
                    payload_threads = error.threads
                    raise
                outcome, observed_status = monitor(service, payload, execution_deadline)
        except (OSError, ValueError, stage1_process.OutputPumpStartError) as error:
            log_line(log, f"launch_error={error}")
        finally:
            owned = []
            if service is not None:
                owned.append((service, service_threads))
            if payload is not None:
                owned.append((payload, payload_threads))
            cleanup_deadline = time.monotonic() + args.cleanup_seconds
            cleanup_succeeded = stage1_process.finish_streaming_processes(
                tuple(owned),
                cleanup_deadline=cleanup_deadline,
                graceful=True,
            )
            write_output(args.service_stdout, bytes(service_stdout))
            write_output(args.service_stderr, bytes(service_stderr))
            write_output(args.payload_stdout, bytes(payload_stdout))
            write_output(args.payload_stderr, bytes(payload_stderr))

        elapsed = time.monotonic() - started
        log_line(log, f"outcome={outcome}")
        log_line(log, f"observed_status={observed_status}")
        log_line(log, f"cleanup_confirmed={str(cleanup_succeeded).lower()}")
        log_line(log, f"elapsed_seconds={elapsed:.3f}")

    if outcome == "timed_out":
        return 124
    if not cleanup_succeeded:
        return 1
    if outcome == "service_exited":
        return 1
    if outcome == "payload_exited" and observed_status is not None:
        return observed_status
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
