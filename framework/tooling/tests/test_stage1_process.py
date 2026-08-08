from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import threading
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).parents[1]))
import stage1_process  # noqa: E402


FIXTURE = Path(__file__).parent / "fixtures" / "process_tree.py"


def command(mode: str, *arguments: str) -> list[str]:
    return [sys.executable, str(FIXTURE), mode, *arguments]


def process_exists(process_id: int) -> bool:
    try:
        os.kill(process_id, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    if sys.platform == "linux":
        try:
            stat = Path(f"/proc/{process_id}/stat").read_text(encoding="utf-8")
        except OSError:
            return False
        state = stat[stat.rfind(")") + 2 :].split(maxsplit=1)[0]
        return state != "Z"
    return True


@unittest.skipIf(os.name == "nt", "POSIX process-group integration tests")
class Stage1ProcessTests(unittest.TestCase):
    def test_success_preserves_complete_output_and_exit_code(self) -> None:
        result = stage1_process.run_capture(command("success"), timeout_seconds=5)

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"stage1 stdout complete\n")
        self.assertEqual(result.stderr, b"stage1 stderr complete\n")
        self.assertFalse(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)

    def test_stdin_environment_and_working_directory_are_forwarded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "stdin.txt"
            input_path.write_text("input payload", encoding="utf-8")
            environment = os.environ | {"STAGE1_PROCESS_VALUE": "environment payload"}
            script = (
                "import os, pathlib, sys; "
                "print(pathlib.Path.cwd().name); "
                "print(os.environ['STAGE1_PROCESS_VALUE']); "
                "print(sys.stdin.read())"
            )
            with input_path.open("rb") as stdin:
                result = stage1_process.run_capture(
                    [sys.executable, "-c", script],
                    timeout_seconds=5,
                    stdin=stdin,
                    environment=environment,
                    cwd=root,
                )

        self.assertEqual(
            result.stdout.decode("utf-8").splitlines(),
            [root.name, "environment payload", "input payload"],
        )
        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.cleanup_timed_out)

    def test_declared_nonzero_status_is_preserved(self) -> None:
        result = stage1_process.run_capture(
            command("exit", "--exit-code", "7"), timeout_seconds=5
        )

        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, b"declared exit 7\n")
        self.assertFalse(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)

    def test_output_pump_start_failure_terminates_owned_process(self) -> None:
        process = stage1_process.start_process(command("spawn-wait"))
        real_start = threading.Thread.start
        starts = 0

        def fail_second_start(thread: threading.Thread) -> None:
            nonlocal starts
            starts += 1
            if starts == 2:
                raise RuntimeError("injected thread-start failure")
            real_start(thread)

        with mock.patch.object(threading.Thread, "start", fail_second_start):
            with self.assertRaisesRegex(RuntimeError, "injected thread-start failure"):
                stage1_process.start_output_pumps(process)

        self.assertIsNotNone(process.returncode)

    def test_process_start_time_counts_toward_timeout(self) -> None:
        real_start_process = stage1_process.start_process

        def delayed_start(*args: object, **kwargs: object) -> stage1_process.ManagedProcess:
            time.sleep(0.1)
            return real_start_process(*args, **kwargs)  # type: ignore[arg-type]

        started = time.monotonic()
        with mock.patch.object(stage1_process, "start_process", side_effect=delayed_start):
            result = stage1_process.run_capture(
                command("success"),
                timeout_seconds=0.05,
                cleanup_seconds=1,
            )
        elapsed = time.monotonic() - started

        self.assertTrue(result.timed_out)
        self.assertLess(elapsed, 0.5)

    def test_run_capture_rejects_unmanaged_stdin_pipe(self) -> None:
        with self.assertRaisesRegex(ValueError, "does not own pipe input"):
            stage1_process.run_capture(
                command("success"),
                timeout_seconds=1,
                stdin=subprocess.PIPE,
            )

    def test_timeout_kills_process_group_with_bounded_cleanup(self) -> None:
        started = time.monotonic()
        result = stage1_process.run_capture(
            command("spawn-wait"),
            timeout_seconds=0.2,
            cleanup_seconds=1,
        )
        elapsed = time.monotonic() - started

        self.assertTrue(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)
        self.assertLess(elapsed, 1.5)
        self.assertFalse(
            any(
                thread.name.startswith("stage1-") and thread.is_alive()
                for thread in threading.enumerate()
            )
        )

    def test_root_exit_kills_descendant_that_retains_output_pipe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "grandchild.pid"
            started = time.monotonic()
            result = stage1_process.run_capture(
                command("spawn-exit", "--pid-file", str(pid_file)),
                timeout_seconds=5,
                cleanup_seconds=1,
            )
            elapsed = time.monotonic() - started
            process_id = int(pid_file.read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.timed_out)
        self.assertFalse(result.cleanup_timed_out)
        self.assertLess(elapsed, 1.5)
        deadline = time.monotonic() + 1
        while process_exists(process_id) and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertFalse(process_exists(process_id))


if __name__ == "__main__":
    unittest.main()
