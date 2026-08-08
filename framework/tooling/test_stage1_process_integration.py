#!/usr/bin/env python3
# pyright: reportAttributeAccessIssue=false
"""Real Windows integration proof for Stage 1 Job Object supervision."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import stage1_process  # noqa: E402


TOOLING = Path(__file__).parent
FIXTURE = TOOLING / "tests" / "fixtures" / "process_tree.py"
WATCHDOG = TOOLING / "stage1_watchdog.py"
PROCESS_QUERY_LIMITED_INFORMATION = 0x00001000
SYNCHRONIZE = 0x00100000
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 258


def command(mode: str, *arguments: str) -> list[str]:
    return [sys.executable, str(FIXTURE), mode, *arguments]


def process_running(process_id: int) -> bool:
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.WaitForSingleObject.restype = wintypes.DWORD
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    handle = kernel32.OpenProcess(SYNCHRONIZE, False, process_id)
    if not handle:
        return False
    try:
        return kernel32.WaitForSingleObject(handle, 0) == WAIT_TIMEOUT
    finally:
        kernel32.CloseHandle(handle)


def wait_for_pid(path: Path, timeout: float = 5) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return int(path.read_text(encoding="utf-8"))
        time.sleep(0.01)
    raise AssertionError(f"process fixture did not write {path}")


@unittest.skipUnless(os.name == "nt", "requires real Windows Job Objects")
class WindowsProcessIntegrationTests(unittest.TestCase):
    def test_prestart_membership_and_handle_allowlist(self) -> None:
        import msvcrt

        with tempfile.NamedTemporaryFile() as unrelated:
            os.set_inheritable(unrelated.fileno(), True)
            environment = os.environ.copy()
            environment["STAGE1_UNRELATED_HANDLE"] = str(
                msvcrt.get_osfhandle(unrelated.fileno())
            )
            result = stage1_process.run_capture(
                command("membership"),
                timeout_seconds=5,
                environment=environment,
            )

        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)
        self.assertEqual(
            json.loads(result.stdout),
            {"job_member": True, "unrelated_handle_inherited": False},
        )

    def test_job_close_terminates_child_and_grandchild(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "grandchild.pid"
            process = stage1_process.start_process(
                command("spawn-wait", "--pid-file", str(pid_file))
            )
            grandchild_pid = wait_for_pid(pid_file)
            self.assertTrue(process_running(process.pid))
            self.assertTrue(process_running(grandchild_pid))

            self.assertTrue(  # type: ignore[attr-defined]
                process.terminate_tree(deadline=time.monotonic() + 2)
            )
            process.wait(timeout=2)
            stage1_process.close_process(process)

            self.assertFalse(process_running(process.pid))
            self.assertFalse(process_running(grandchild_pid))

    def test_root_exit_cannot_leave_descendant_holding_stdout(self) -> None:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.IsProcessInJob.argtypes = [
            wintypes.HANDLE,
            wintypes.HANDLE,
            ctypes.POINTER(wintypes.BOOL),
        ]
        kernel32.IsProcessInJob.restype = wintypes.BOOL
        kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel32.WaitForSingleObject.restype = wintypes.DWORD
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_file = root / "grandchild.pid"
            release_file = root / "release-root"
            started = time.monotonic()
            process = stage1_process.start_process(
                command(
                    "spawn-exit",
                    "--pid-file",
                    str(pid_file),
                    "--release-file",
                    str(release_file),
                )
            )
            _, _, threads = stage1_process.start_output_pumps(process)
            grandchild_handle = None
            cleanup_finished = False
            try:
                grandchild_pid = wait_for_pid(pid_file)
                grandchild_handle = kernel32.OpenProcess(
                    SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                    False,
                    grandchild_pid,
                )
                self.assertTrue(grandchild_handle)
                in_job = wintypes.BOOL()
                self.assertTrue(
                    kernel32.IsProcessInJob(
                        grandchild_handle,
                        process._stage1_job_handle,  # type: ignore[attr-defined]
                        ctypes.byref(in_job),
                    )
                )
                self.assertTrue(in_job.value)
                self.assertIsNone(process.poll())

                release_file.touch()
                self.assertTrue(
                    stage1_process.observe_process_exit(
                        process,
                        deadline=started + 5,
                    )
                )
                self.assertEqual(process.returncode, 0)
                self.assertTrue(any(thread.is_alive() for thread in threads))
                cleanup_finished = stage1_process.finish_streaming_process(
                    process,
                    threads,
                    cleanup_deadline=time.monotonic() + 2,
                )
                self.assertTrue(cleanup_finished)
                self.assertEqual(
                    kernel32.WaitForSingleObject(grandchild_handle, 0),
                    WAIT_OBJECT_0,
                )
                self.assertLess(time.monotonic() - started, 7)
            finally:
                if not cleanup_finished:
                    stage1_process.finish_streaming_process(
                        process,
                        threads,
                        cleanup_deadline=time.monotonic() + 2,
                    )
                if grandchild_handle:
                    kernel32.CloseHandle(grandchild_handle)

    def test_file_and_pipe_stdin_environment_and_cwd_are_supported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "input.txt"
            input_path.write_text("file payload", encoding="utf-8")
            script = (
                "import os, pathlib, sys; "
                "print(pathlib.Path.cwd().name); "
                "print(os.environ['STAGE1_PROCESS_VALUE']); "
                "print(sys.stdin.read())"
            )
            environment = os.environ | {"STAGE1_PROCESS_VALUE": "environment payload"}
            with input_path.open("rb") as stdin:
                process = stage1_process.start_process(
                    [sys.executable, "-c", script],
                    stdin=stdin,
                    environment=environment,
                    cwd=root,
                )
                stdout, _, threads = stage1_process.start_output_pumps(process)
                deadline = time.monotonic() + 5
                self.assertTrue(
                    stage1_process.observe_process_exit(process, deadline=deadline)
                )
                self.assertTrue(
                    stage1_process.finish_streaming_process(
                        process,
                        threads,
                        cleanup_deadline=deadline,
                    )
                )
            self.assertEqual(
                bytes(stdout).decode("utf-8").splitlines(),
                [root.name, "environment payload", "file payload"],
            )

            process = stage1_process.start_process(
                [sys.executable, "-c", "import sys; print(sys.stdin.read())"],
                stdin=subprocess.PIPE,
            )
            stdout, _, threads = stage1_process.start_output_pumps(process)
            assert process.stdin is not None
            process.stdin.write(b"pipe payload")
            process.stdin.close()
            deadline = time.monotonic() + 5
            self.assertTrue(stage1_process.observe_process_exit(process, deadline=deadline))
            self.assertTrue(
                stage1_process.finish_streaming_process(
                    process,
                    threads,
                    cleanup_deadline=deadline,
                )
            )
            self.assertEqual(bytes(stdout), b"pipe payload\r\n")

    def test_success_and_declared_nonzero_preserve_output(self) -> None:
        success = stage1_process.run_capture(command("success"), timeout_seconds=5)
        nonzero = stage1_process.run_capture(
            command("exit", "--exit-code", "7"), timeout_seconds=5
        )

        self.assertEqual(success.returncode, 0)
        line_ending = os.linesep.encode()
        self.assertEqual(success.stdout, b"stage1 stdout complete" + line_ending)
        self.assertEqual(success.stderr, b"stage1 stderr complete" + line_ending)
        self.assertEqual(nonzero.returncode, 7)
        self.assertEqual(nonzero.stdout, b"declared exit 7" + line_ending)
        self.assertFalse(success.cleanup_timed_out)
        self.assertFalse(nonzero.cleanup_timed_out)

    def test_timeout_is_bounded_by_cleanup_allowance(self) -> None:
        started = time.monotonic()
        result = stage1_process.run_capture(
            command("spawn-wait"),
            timeout_seconds=0.25,
            cleanup_seconds=2,
        )
        elapsed = time.monotonic() - started

        self.assertTrue(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)
        self.assertLess(elapsed, 2.5)

    def test_watchdog_returns_124_and_accepts_declared_nonzero(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def run_watchdog(name: str, command: list[str], *options: str) -> tuple[int, float]:
                started = time.monotonic()
                result = subprocess.run(
                    [
                        sys.executable,
                        str(WATCHDOG),
                        "--timeout-seconds",
                        "0.25",
                        "--cleanup-seconds",
                        "2",
                        "--stdout",
                        str(root / f"{name}.stdout"),
                        "--stderr",
                        str(root / f"{name}.stderr"),
                        "--log",
                        str(root / f"{name}.log"),
                        *options,
                        "--",
                        *command,
                    ],
                    check=False,
                    timeout=5,
                )
                return result.returncode, time.monotonic() - started

            timeout_code, timeout_elapsed = run_watchdog("timeout", command("spawn-wait"))
            expected_code, _ = run_watchdog(
                "nonzero",
                command("exit", "--exit-code", "7"),
                "--expected-exit-code",
                "7",
            )

        self.assertEqual(timeout_code, 124)
        self.assertLess(timeout_elapsed, 2.5)
        self.assertEqual(expected_code, 0)


if __name__ == "__main__":
    unittest.main()
