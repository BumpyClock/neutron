#!/usr/bin/env python3
"""Prove native clipboard output with an external reader and TCP acknowledgement."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import queue
import shlex
import socket
import subprocess
import sys
import threading
import time
from typing import BinaryIO, TextIO

import stage1_process


class HarnessError(Exception):
    pass


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
        description=(
            "Run the clipboard conformance scenario, read its OS clipboard output, "
            "acknowledge it over loopback TCP, and validate the resulting JSONL."
        ),
        allow_abbrev=False,
    )
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--reader-timeout-seconds", type=float, default=30)
    parser.add_argument("--validation-timeout-seconds", type=float, default=30)
    parser.add_argument("--validation-profile", required=True)
    parser.add_argument("--stdout", type=Path, required=True)
    parser.add_argument("--stderr", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--reader-stdout", type=Path, required=True)
    parser.add_argument("--reader-stderr", type=Path, required=True)
    parser.add_argument("--validation-stdout", type=Path, required=True)
    parser.add_argument("--validation-stderr", type=Path, required=True)
    parser.add_argument("--validation-log", type=Path, required=True)
    parser.add_argument("--reader-command", nargs=argparse.REMAINDER, required=True)
    args = parser.parse_args()

    for name in (
        "timeout_seconds",
        "reader_timeout_seconds",
        "validation_timeout_seconds",
    ):
        if getattr(args, name) <= 0:
            parser.error(f"--{name.replace('_', '-')} must be greater than zero")
    if not args.reader_command:
        parser.error("--reader-command requires a command")
    if args.reader_command[:1] == ["--"]:
        args.reader_command = args.reader_command[1:]
    if not args.reader_command:
        parser.error("--reader-command requires a command after --")
    return args


def prepare_output(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"")


def start_process(
    command: list[str],
    *,
    stdin: BinaryIO | int = subprocess.DEVNULL,
) -> stage1_process.ManagedProcess:
    return stage1_process.start_process(command, stdin=stdin)


def pump_stdout(
    stream: BinaryIO,
    output: BinaryIO,
    records: queue.Queue[bytes | None],
) -> None:
    try:
        for line in iter(stream.readline, b""):
            output.write(line)
            output.flush()
            records.put(line)
    except (OSError, ValueError):
        pass
    finally:
        records.put(None)


def pump_stderr(stream: BinaryIO, output: BinaryIO) -> None:
    try:
        for chunk in iter(lambda: stream.read(65536), b""):
            output.write(chunk)
            output.flush()
    except (OSError, ValueError):
        pass


def remaining(deadline: float) -> float:
    value = deadline - time.monotonic()
    if value <= 0:
        raise HarnessError("clipboard scenario exceeded its external timeout")
    return value


def parse_scenario_record(line: bytes, phase: str) -> dict[str, object]:
    try:
        record = json.loads(line)
    except json.JSONDecodeError as error:
        raise HarnessError(f"scenario wrote invalid JSONL {phase}: {error}") from error
    if not isinstance(record, dict):
        raise HarnessError(f"scenario wrote a non-object JSONL record {phase}")
    if not isinstance(record.get("event"), str):
        raise HarnessError(f"scenario wrote a JSONL record without a string event {phase}")
    return record


def parse_clipboard_ready_record(record: dict[str, object]) -> tuple[bytes, str] | None:
    if record.get("event") != "clipboard_ready":
        return None
    data = record.get("data")
    if not isinstance(data, dict):
        raise HarnessError("clipboard_ready data was not an object")
    payload = data.get("expected_payload")
    address = data.get("ack_address")
    if not isinstance(payload, str) or not payload:
        raise HarnessError("clipboard_ready expected_payload was not a nonempty string")
    if not isinstance(address, str):
        raise HarnessError("clipboard_ready ack_address was not a string")

    try:
        payload_bytes = payload.encode("utf-8")
    except UnicodeEncodeError as error:
        raise HarnessError("clipboard_ready expected_payload was not valid UTF-8") from error
    if payload_bytes.endswith((b"\n", b"\r")):
        raise HarnessError("clipboard_ready expected_payload must not end with a line break")
    return payload_bytes, address


def reject_premature_completion(record: dict[str, object], phase: str) -> None:
    event = record["event"]
    if event in (
        "clipboard_acknowledged",
        "quit_requested",
        "shutdown_started",
        "will_exit",
        "shutdown_complete",
        "run_returned",
        "terminal",
    ):
        raise HarnessError(f"clipboard scenario emitted {event} {phase}")
    if event == "app_event":
        data = record.get("data")
        if isinstance(data, dict) and data.get("kind") in (
            "shutdown_requested",
            "will_exit",
        ):
            raise HarnessError(f"clipboard scenario began shutdown {phase}")


def wait_for_clipboard_ready(
    process: stage1_process.ManagedProcess,
    records: queue.Queue[bytes | None],
    deadline: float,
) -> tuple[bytes, str]:
    while True:
        try:
            line = records.get(timeout=min(remaining(deadline), 0.25))
        except queue.Empty:
            if process.poll() is not None:
                raise HarnessError(
                    f"clipboard scenario exited with status {process.returncode} before clipboard_ready"
                )
            continue
        if line is None:
            raise HarnessError("clipboard scenario closed stdout before clipboard_ready")
        record = parse_scenario_record(line, "before clipboard readiness")
        reject_premature_completion(record, "before clipboard_ready")
        ready = parse_clipboard_ready_record(record)
        if ready is not None:
            return ready


def require_process_running(process: stage1_process.ManagedProcess, phase: str) -> None:
    if process.poll() is not None:
        raise HarnessError(
            f"clipboard scenario exited with status {process.returncode} {phase}"
        )


def require_scenario_active(
    process: stage1_process.ManagedProcess,
    records: queue.Queue[bytes | None],
    phase: str,
) -> None:
    require_process_running(process, phase)
    while True:
        try:
            line = records.get_nowait()
        except queue.Empty:
            break
        if line is None:
            raise HarnessError(f"clipboard scenario closed stdout {phase}")
        record = parse_scenario_record(line, phase)
        event = record.get("event")
        if event == "clipboard_ready":
            raise HarnessError(f"clipboard scenario emitted a duplicate clipboard_ready {phase}")
        reject_premature_completion(record, phase)
    require_process_running(process, phase)


def normalize_reader_output(output: bytes) -> bytes:
    normalized = output.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    if normalized.endswith(b"\n"):
        normalized = normalized[:-1]
    return normalized


def run_reader(
    command: list[str],
    timeout_seconds: float,
    cleanup_deadline: float,
    stdout_path: Path,
    stderr_path: Path,
    log: TextIO,
) -> bytes:
    log_line(log, f"clipboard_reader_command={command_text(command)}")
    try:
        result = stage1_process.run_capture(
            command,
            timeout_seconds=timeout_seconds,
            cleanup_deadline=cleanup_deadline,
        )
    except OSError as error:
        raise HarnessError(f"could not start clipboard reader: {error}") from error

    stdout_path.write_bytes(result.stdout)
    stderr_path.write_bytes(result.stderr)
    log_line(
        log,
        f"clipboard_reader_exit_code={result.returncode} "
        f"stdout_bytes={len(result.stdout)} stderr_bytes={len(result.stderr)}",
    )
    if result.timed_out:
        raise HarnessError(f"clipboard reader exceeded {timeout_seconds:g} seconds")
    if result.cleanup_timed_out:
        raise HarnessError("clipboard reader cleanup exceeded its hard allowance")
    if result.returncode != 0:
        raise HarnessError(f"clipboard reader exited with status {result.returncode}")
    return result.stdout


def parse_loopback_address(address: str) -> tuple[str, int]:
    host, separator, port_text = address.rpartition(":")
    if not separator or not host or not port_text:
        raise HarnessError("clipboard_ready ack_address must be host:port")
    try:
        parsed_host = ipaddress.ip_address(host)
    except ValueError as error:
        raise HarnessError("clipboard_ready ack_address host was not an IP address") from error
    if str(parsed_host) != "127.0.0.1":
        raise HarnessError("clipboard_ready ack_address must use 127.0.0.1")
    if not port_text.isascii() or not port_text.isdecimal():
        raise HarnessError("clipboard_ready ack_address port was not an ASCII decimal integer")
    try:
        port = int(port_text)
    except ValueError as error:
        raise HarnessError("clipboard_ready ack_address port was not an integer") from error
    if not 1 <= port <= 65535:
        raise HarnessError("clipboard_ready ack_address port was outside 1..65535")
    return host, port


def acknowledge(address: str, timeout_seconds: float, log: TextIO) -> None:
    host, port = parse_loopback_address(address)
    try:
        with socket.create_connection((host, port), timeout=timeout_seconds) as connection:
            connection.sendall(b"verified\n")
            connection.shutdown(socket.SHUT_WR)
    except OSError as error:
        raise HarnessError(f"could not acknowledge external clipboard verification: {error}") from error
    log_line(log, f"clipboard_acknowledged={host}:{port}")


def wait_for_exit(
    process: stage1_process.ManagedProcess, deadline: float, log: TextIO
) -> None:
    if not stage1_process.observe_process_exit(process, deadline=deadline):
        raise HarnessError("clipboard scenario did not return after verification")
    returncode = process.poll()
    log_line(log, f"clipboard_scenario_exit_code={returncode}")
    if returncode != 0:
        raise HarnessError(f"clipboard scenario exited with status {returncode}")


def validate_trace(
    binary: Path,
    stdout_path: Path,
    timeout_seconds: float,
    validation_profile: str,
    stdout_output: Path,
    stderr_output: Path,
    log_output: Path,
) -> None:
    command = [str(binary), "--validate", "clipboard", "--profile", validation_profile]
    for path in (stdout_output, stderr_output, log_output):
        path.parent.mkdir(parents=True, exist_ok=True)
    with log_output.open("w", encoding="utf-8") as log:
        log_line(log, f"validator_command={command_text(command)}")
        with stdout_path.open("rb") as trace:
            try:
                result = stage1_process.run_capture(
                    command,
                    timeout_seconds=timeout_seconds,
                    stdin=trace,
                )
            except OSError as error:
                raise HarnessError(f"could not start conformance validator: {error}") from error

        stdout_output.write_bytes(result.stdout)
        stderr_output.write_bytes(result.stderr)
        log_line(
            log,
            f"validator_exit_code={result.returncode} "
            f"stdout_bytes={len(result.stdout)} stderr_bytes={len(result.stderr)}",
        )
        if result.timed_out:
            log_line(log, "validator timed out")
            raise HarnessError(f"conformance validator exceeded {timeout_seconds:g} seconds")
        if result.cleanup_timed_out:
            raise HarnessError("conformance validator cleanup exceeded its hard allowance")
        if result.returncode != 0:
            raise HarnessError(f"conformance validator exited with status {result.returncode}")
        if result.stdout:
            raise HarnessError("conformance validator wrote unexpected stdout")


def assert_orderly_clipboard_trace(stdout_path: Path) -> None:
    records = []
    for line_number, line in enumerate(stdout_path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise HarnessError(f"captured JSONL line {line_number} was invalid: {error}") from error

    ready_indices = [
        index for index, record in enumerate(records) if record.get("event") == "clipboard_ready"
    ]
    if len(ready_indices) != 1:
        raise HarnessError(
            f"clipboard trace contained {len(ready_indices)} clipboard_ready records; expected one"
        )
    acknowledgement_indices = [
        index
        for index, record in enumerate(records)
        if record.get("event") == "clipboard_acknowledged"
    ]
    if len(acknowledgement_indices) != 1:
        raise HarnessError(
            "clipboard trace must contain exactly one clipboard_acknowledged record"
        )
    terminal_indices = [
        index for index, record in enumerate(records) if record.get("event") == "terminal"
    ]
    if len(terminal_indices) != 1:
        raise HarnessError("clipboard trace must contain exactly one terminal record")
    if terminal_indices[0] <= acknowledgement_indices[0]:
        raise HarnessError("clipboard terminal must follow external acknowledgement")
    if not records or records[-1].get("event") != "terminal":
        raise HarnessError("clipboard trace did not end with terminal")
    terminal = records[-1].get("data")
    if (
        not isinstance(terminal, dict)
        or terminal.get("outcome") != "passed"
        or terminal.get("exit_code") != 0
    ):
        raise HarnessError("clipboard trace did not terminate with passed/0")
    run_returned_index = next(
        (
            index
            for index in range(len(records) - 2, -1, -1)
            if records[index].get("event") == "run_returned"
        ),
        None,
    )
    if run_returned_index is None or records[run_returned_index].get("data", {}).get("result") != "ok":
        raise HarnessError("clipboard trace did not report run_returned result ok before terminal")
    if not ready_indices[0] < acknowledgement_indices[0] < run_returned_index:
        raise HarnessError(
            "clipboard_ready, clipboard_acknowledged, and run_returned were not ordered"
        )


def main() -> int:
    args = parse_args()
    for path in (
        args.stdout,
        args.stderr,
        args.reader_stdout,
        args.reader_stderr,
        args.validation_stdout,
        args.validation_stderr,
    ):
        prepare_output(path)
    args.log.parent.mkdir(parents=True, exist_ok=True)

    process = None
    stdout_thread = None
    stderr_thread = None
    started_threads: list[threading.Thread] = []
    cleanup_attempted = False
    cleanup_succeeded = False
    cleanup_deadline = None
    with args.log.open("w", encoding="utf-8") as log:
        command = [str(args.binary), "--scenario", "clipboard"]
        log_line(log, f"clipboard_scenario_command={command_text(command)}")
        log_line(log, f"timeout_seconds={args.timeout_seconds:g}")
        deadline = time.monotonic() + args.timeout_seconds
        cleanup_deadline = deadline + stage1_process.DEFAULT_CLEANUP_SECONDS
        try:
            process = start_process(command)
            assert process.stdout is not None
            assert process.stderr is not None
            records: queue.Queue[bytes | None] = queue.Queue()
            with args.stdout.open("wb") as stdout, args.stderr.open("wb") as stderr:
                try:
                    stdout_thread = threading.Thread(
                        target=pump_stdout,
                        args=(process.stdout, stdout, records),
                        daemon=True,
                    )
                    stderr_thread = threading.Thread(
                        target=pump_stderr,
                        args=(process.stderr, stderr),
                        daemon=True,
                    )
                    stdout_thread.start()
                    started_threads.append(stdout_thread)
                    stderr_thread.start()
                    started_threads.append(stderr_thread)

                    expected_payload, acknowledgement_address = wait_for_clipboard_ready(
                        process, records, deadline
                    )
                    log_line(
                        log,
                        "clipboard_ready "
                        f"payload_bytes={len(expected_payload)} payload_sha256={hashlib.sha256(expected_payload).hexdigest()}",
                    )
                    parse_loopback_address(acknowledgement_address)
                    require_scenario_active(
                        process, records, "before external clipboard reading"
                    )
                    reader_output = run_reader(
                        args.reader_command,
                        min(args.reader_timeout_seconds, remaining(deadline)),
                        cleanup_deadline,
                        args.reader_stdout,
                        args.reader_stderr,
                        log,
                    )
                    normalized_reader_output = normalize_reader_output(reader_output)
                    log_line(
                        log,
                        "clipboard_reader_normalized "
                        f"bytes={len(normalized_reader_output)} "
                        f"sha256={hashlib.sha256(normalized_reader_output).hexdigest()}",
                    )
                    if normalized_reader_output != expected_payload:
                        raise HarnessError("external clipboard reader output did not match expected payload")
                    require_scenario_active(
                        process, records, "before external clipboard acknowledgement"
                    )
                    acknowledge(acknowledgement_address, remaining(deadline), log)
                    wait_for_exit(process, deadline, log)
                finally:
                    cleanup_attempted = True
                    cleanup_succeeded = stage1_process.finish_streaming_process(
                        process,
                        tuple(started_threads),
                        cleanup_deadline=cleanup_deadline,
                    )

                if not cleanup_succeeded:
                    raise HarnessError(
                        "clipboard scenario cleanup exceeded its hard allowance"
                    )

            assert_orderly_clipboard_trace(args.stdout)
            validate_trace(
                args.binary,
                args.stdout,
                args.validation_timeout_seconds,
                args.validation_profile,
                args.validation_stdout,
                args.validation_stderr,
                args.validation_log,
            )
            log_line(log, "clipboard conformance succeeded")
            return 0
        except (HarnessError, OSError) as error:
            detail = str(error)
            if cleanup_attempted and not cleanup_succeeded:
                detail = f"{detail}; clipboard scenario cleanup was not confirmed"
            log_line(log, f"clipboard conformance failed: {detail}")
            return 1
        finally:
            if process is not None and not cleanup_attempted:
                cleanup_succeeded = stage1_process.finish_streaming_process(
                    process,
                    tuple(started_threads),
                    cleanup_deadline=cleanup_deadline,
                )
                if not cleanup_succeeded:
                    log_line(
                        log,
                        "clipboard conformance cleanup failed before scenario execution completed",
                    )


if __name__ == "__main__":
    sys.exit(main())
