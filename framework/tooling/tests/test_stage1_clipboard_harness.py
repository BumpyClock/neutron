from __future__ import annotations

import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
TOOLING = REPOSITORY_ROOT / "tooling"
sys.path.insert(0, str(TOOLING))
HARNESS = TOOLING / "stage1_clipboard_harness.py"
SCENARIO = Path(__file__).parent / "fixtures" / "fake_clipboard_scenario.py"
READER = Path(__file__).parent / "fixtures" / "fake_clipboard_reader.py"
HARNESS_SPEC = importlib.util.spec_from_file_location("stage1_clipboard_harness", HARNESS)
assert HARNESS_SPEC is not None and HARNESS_SPEC.loader is not None
HARNESS_MODULE = importlib.util.module_from_spec(HARNESS_SPEC)
HARNESS_SPEC.loader.exec_module(HARNESS_MODULE)


@unittest.skipUnless(os.name == "posix", "requires POSIX process-group cleanup")
class ClipboardHarnessTests(unittest.TestCase):
    def run_harness(
        self, mode: str
    ) -> tuple[subprocess.CompletedProcess[str], Path, str, int, list[dict[str, object]]]:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        directory = Path(temporary_directory.name)
        marker = directory / "acknowledged"
        stdout = directory / "scenario.stdout"
        environment = os.environ | {
            "STAGE1_CLIPBOARD_FIXTURE_MODE": mode,
            "STAGE1_CLIPBOARD_FIXTURE_ACK_MARKER": str(marker),
        }
        command = [
            sys.executable,
            str(HARNESS),
            "--binary",
            str(SCENARIO),
            "--timeout-seconds",
            "1",
            "--reader-timeout-seconds",
            "1",
            "--validation-timeout-seconds",
            "1",
            "--validation-profile",
            "macos-metal",
            "--stdout",
            str(stdout),
            "--stderr",
            str(directory / "scenario.stderr"),
            "--log",
            str(directory / "harness.log"),
            "--reader-stdout",
            str(directory / "reader.stdout"),
            "--reader-stderr",
            str(directory / "reader.stderr"),
            "--validation-stdout",
            str(directory / "validation.stdout"),
            "--validation-stderr",
            str(directory / "validation.stderr"),
            "--validation-log",
            str(directory / "validation.log"),
            "--reader-command",
            sys.executable,
            str(READER),
        ]
        result = subprocess.run(
            command,
            capture_output=True,
            encoding="utf-8",
            env=environment,
            timeout=5,
        )
        if not stdout.exists():
            self.fail(
                f"harness did not create scenario stdout: stdout={result.stdout!r} stderr={result.stderr!r}"
            )
        records = [json.loads(line) for line in stdout.read_text(encoding="utf-8").splitlines()]
        ready = next(record for record in records if record["event"] == "clipboard_ready")
        host, port_text = ready["data"]["ack_address"].rsplit(":", 1)
        return result, marker, host, int(port_text), records

    def assert_listener_closed(self, host: str, port: int) -> None:
        with self.assertRaises(OSError):
            socket.create_connection((host, port), timeout=1)

    def test_accepts_orderly_return_after_acknowledgement(self) -> None:
        result, _, host, port, _ = self.run_harness("orderly")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_listener_closed(host, port)

    def test_rejects_terminal_before_external_acknowledgement(self) -> None:
        result, marker, host, port, _ = self.run_harness("premature-terminal")

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(marker.exists(), "harness sent verified after terminal")
        self.assert_listener_closed(host, port)

    def test_cleans_listener_after_root_exits(self) -> None:
        result, marker, host, port, _ = self.run_harness("root-exits")

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(marker.exists(), "harness sent verified after root exit")
        self.assert_listener_closed(host, port)

    def test_rejects_descendant_false_trace(self) -> None:
        result, marker, host, port, records = self.run_harness("false-trace-root-exits")

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(marker.exists(), "harness sent verified to the descendant listener")
        self.assert_listener_closed(host, port)
        self.assertIn("clipboard_acknowledged", [record["event"] for record in records])
        self.assertIn("terminal", [record["event"] for record in records])


class ClipboardCleanupReportingTests(unittest.TestCase):
    def test_reports_unconfirmed_cleanup_with_primary_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory_name:
            directory = Path(directory_name)
            args = SimpleNamespace(
                binary=directory / "scenario",
                timeout_seconds=1.0,
                reader_timeout_seconds=1.0,
                validation_timeout_seconds=1.0,
                validation_profile="macos-metal",
                stdout=directory / "scenario.stdout",
                stderr=directory / "scenario.stderr",
                log=directory / "harness.log",
                reader_stdout=directory / "reader.stdout",
                reader_stderr=directory / "reader.stderr",
                validation_stdout=directory / "validation.stdout",
                validation_stderr=directory / "validation.stderr",
                validation_log=directory / "validation.log",
                reader_command=["reader"],
            )
            process = SimpleNamespace(stdout=io.BytesIO(), stderr=io.BytesIO())

            with (
                mock.patch.object(HARNESS_MODULE, "parse_args", return_value=args),
                mock.patch.object(HARNESS_MODULE, "start_process", return_value=process),
                mock.patch.object(
                    HARNESS_MODULE.stage1_process,
                    "finish_streaming_process",
                    return_value=False,
                ),
            ):
                self.assertEqual(HARNESS_MODULE.main(), 1)

            self.assertIn(
                "clipboard scenario cleanup was not confirmed",
                args.log.read_text(encoding="utf-8"),
            )


class ClipboardReadyTests(unittest.TestCase):
    def test_rejects_non_utf8_expected_payload(self) -> None:
        record = {
            "event": "clipboard_ready",
            "data": {
                "expected_payload": "\ud800",
                "ack_address": "127.0.0.1:1",
            },
        }

        with self.assertRaises(HARNESS_MODULE.HarnessError):
            HARNESS_MODULE.parse_clipboard_ready_record(record)


class ClipboardAddressTests(unittest.TestCase):
    def test_rejects_noncanonical_port_text(self) -> None:
        for address in (
            "127.0.0.1:+1",
            "127.0.0.1: 1",
            "127.0.0.1:1 ",
            "127.0.0.1:١",
        ):
            with self.subTest(address=address):
                with self.assertRaises(HARNESS_MODULE.HarnessError):
                    HARNESS_MODULE.parse_loopback_address(address)


class ClipboardTraceTests(unittest.TestCase):
    def test_rejects_terminal_before_acknowledgement(self) -> None:
        records = [
            {
                "event": "clipboard_ready",
                "data": {
                    "expected_payload": "synthetic clipboard payload",
                    "ack_address": "127.0.0.1:1",
                },
            },
            {"event": "terminal", "data": {"outcome": "passed", "exit_code": 0}},
            {"event": "clipboard_acknowledged", "data": {}},
            {"event": "run_returned", "data": {"result": "ok"}},
            {"event": "terminal", "data": {"outcome": "passed", "exit_code": 0}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / "trace.jsonl"
            trace.write_text(
                "".join(f"{json.dumps(record)}\n" for record in records), encoding="utf-8"
            )

            with self.assertRaises(HARNESS_MODULE.HarnessError):
                HARNESS_MODULE.assert_orderly_clipboard_trace(trace)


if __name__ == "__main__":
    unittest.main()
