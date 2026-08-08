#!/usr/bin/env python3
"""Synthetic clipboard scenarios for stage1 harness regression tests."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys


PAYLOAD = "synthetic clipboard payload"


def emit(event: str, data: dict[str, object]) -> None:
    print(json.dumps({"event": event, "data": data}), flush=True)


def run_listener(marker: Path) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        print(listener.getsockname()[1], flush=True)
        while True:
            connection, _ = listener.accept()
            with connection:
                acknowledgement = connection.recv(64)
            if acknowledgement == b"verified\n":
                marker.write_text("verified\n", encoding="utf-8")


def run_orderly_scenario() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        emit(
            "clipboard_ready",
            {
                "expected_payload": PAYLOAD,
                "ack_address": f"127.0.0.1:{listener.getsockname()[1]}",
            },
        )
        connection, _ = listener.accept()
        with connection:
            acknowledgement = connection.recv(64)
    if acknowledgement != b"verified\n":
        return 2
    emit("clipboard_acknowledged", {"acknowledgement": "verified"})
    emit("run_returned", {"result": "ok"})
    emit("terminal", {"outcome": "passed", "exit_code": 0})
    return 0


def run_false_trace_root_exit() -> int:
    marker = Path(os.environ["STAGE1_CLIPBOARD_FIXTURE_ACK_MARKER"])
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        control_read, control_write = os.pipe()
        done_read, done_write = os.pipe()
        child_pid = os.fork()
        if child_pid == 0:
            os.close(control_write)
            os.close(done_read)
            os.read(control_read, 1)
            os.close(control_read)
            emit("clipboard_acknowledged", {"acknowledgement": "verified"})
            emit("run_returned", {"result": "ok"})
            emit("terminal", {"outcome": "passed", "exit_code": 0})
            os.write(done_write, b"1")
            os.close(done_write)
            while True:
                connection, _ = listener.accept()
                with connection:
                    acknowledgement = connection.recv(64)
                if acknowledgement == b"verified\n":
                    marker.write_text("verified\n", encoding="utf-8")

        os.close(control_read)
        os.close(done_write)
        emit(
            "clipboard_ready",
            {
                "expected_payload": PAYLOAD,
                "ack_address": f"127.0.0.1:{listener.getsockname()[1]}",
            },
        )
        os.write(control_write, b"1")
        os.close(control_write)
        os.read(done_read, 1)
        os.close(done_read)
        return 0


def run_scenario() -> int:
    mode = os.environ["STAGE1_CLIPBOARD_FIXTURE_MODE"]
    if mode == "orderly":
        return run_orderly_scenario()
    if mode == "false-trace-root-exits":
        return run_false_trace_root_exit()

    marker = Path(os.environ["STAGE1_CLIPBOARD_FIXTURE_ACK_MARKER"])
    listener = subprocess.Popen(
        [sys.executable, str(Path(__file__)), "--listener", str(marker)],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert listener.stdout is not None
    port_text = listener.stdout.readline().strip()
    if not port_text.isdigit():
        return 2

    emit(
        "clipboard_ready",
        {
            "expected_payload": PAYLOAD,
            "ack_address": f"127.0.0.1:{port_text}",
        },
    )
    if mode == "root-exits":
        return 0
    emit("terminal", {"outcome": "passed", "exit_code": 0})
    if mode == "premature-terminal":
        listener.wait()
        return listener.returncode
    return 2


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--scenario")
    parser.add_argument("--validate")
    parser.add_argument("--profile")
    parser.add_argument("--listener", type=Path)
    args = parser.parse_args()

    if args.listener is not None:
        return run_listener(args.listener)
    if args.scenario == "clipboard":
        return run_scenario()
    if args.validate == "clipboard" and args.profile == "macos-metal":
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
