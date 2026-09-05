from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).parents[1]))
import stage1_process  # noqa: E402
import stage1_watchdog  # noqa: E402


TOOLING = Path(__file__).parents[1]
WATCHDOG = TOOLING / "stage1_watchdog.py"
FIXTURE = Path(__file__).parent / "fixtures" / "process_tree.py"


class Stage1WatchdogTests(unittest.TestCase):
    def watchdog_command(
        self,
        directory: Path,
        command: list[str],
        *options: str,
    ) -> list[str]:
        return [
            sys.executable,
            str(WATCHDOG),
            "--timeout-seconds",
            "2",
            "--cleanup-seconds",
            "1",
            "--stdout",
            str(directory / "stdout.log"),
            "--stderr",
            str(directory / "stderr.log"),
            "--log",
            str(directory / "watchdog.log"),
            *options,
            "--",
            *command,
        ]

    def run_watchdog(
        self,
        directory: Path,
        command: list[str],
        *options: str,
        outer_timeout: float = 10,
    ) -> tuple[subprocess.CompletedProcess[bytes], Path, Path, Path]:
        result = subprocess.run(
            self.watchdog_command(directory, command, *options),
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=outer_timeout,
        )
        return (
            result,
            directory / "stdout.log",
            directory / "stderr.log",
            directory / "watchdog.log",
        )

    def test_success_preserves_complete_output(self) -> None:
        with tempfile.TemporaryDirectory(dir=TOOLING / "tests") as temporary:
            result, stdout, stderr, _ = self.run_watchdog(
                Path(temporary), [sys.executable, str(FIXTURE), "success"]
            )

            self.assertEqual(result.returncode, 0)
            self.assertEqual(stdout.read_bytes(), b"stage1 stdout complete\n")
            self.assertEqual(stderr.read_bytes(), b"stage1 stderr complete\n")

    def test_output_reaches_files_before_child_exit(self) -> None:
        with tempfile.TemporaryDirectory(dir=TOOLING / "tests") as temporary:
            directory = Path(temporary)
            release = directory / "release"
            stdout = directory / "stdout.log"
            stderr = directory / "stderr.log"
            child = (
                "import pathlib, sys, time\n"
                "print('stdout ready', flush=True)\n"
                "print('stderr ready', file=sys.stderr, flush=True)\n"
                "deadline = time.monotonic() + 10\n"
                "while not pathlib.Path(sys.argv[1]).exists():\n"
                "    if time.monotonic() >= deadline: sys.exit(2)\n"
                "    time.sleep(0.01)\n"
                "sys.stdout.buffer.write(b'o' * 131072)\n"
                "sys.stderr.buffer.write(b'e' * 131072)\n"
            )
            process = subprocess.Popen(
                self.watchdog_command(
                    directory,
                    [sys.executable, "-c", child, str(release)],
                    "--timeout-seconds",
                    "5",
                ),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    if (
                        stdout.exists()
                        and stderr.exists()
                        and stdout.read_bytes() == b"stdout ready\n"
                        and stderr.read_bytes() == b"stderr ready\n"
                    ):
                        break
                    time.sleep(0.01)
                self.assertEqual(stdout.read_bytes(), b"stdout ready\n")
                self.assertEqual(stderr.read_bytes(), b"stderr ready\n")
                self.assertIsNone(process.poll())
            finally:
                release.touch()
                diagnostic_stdout, diagnostic_stderr = process.communicate(timeout=8)

            self.assertEqual(
                process.returncode, 0, (diagnostic_stdout, diagnostic_stderr)
            )
            self.assertEqual(stdout.read_bytes(), b"stdout ready\n" + b"o" * 131072)
            self.assertEqual(stderr.read_bytes(), b"stderr ready\n" + b"e" * 131072)
            self.assertIn(
                "stdout_bytes=131085 stderr_bytes=131085",
                (directory / "watchdog.log").read_text(encoding="utf-8"),
            )

    def test_output_error_fails_the_watchdog(self) -> None:
        with tempfile.TemporaryDirectory(dir=TOOLING / "tests") as temporary:
            directory = Path(temporary)
            argv = self.watchdog_command(directory, [sys.executable, "-c", "pass"])
            with (
                mock.patch.object(sys, "argv", argv[1:]),
                mock.patch.object(
                    stage1_process,
                    "run_capture",
                    side_effect=stage1_process.OutputWriteError("injected write failure"),
                ),
            ):
                self.assertEqual(stage1_watchdog.main(), 1)
            self.assertIn(
                "could not preserve command output: injected write failure",
                (directory / "watchdog.log").read_text(encoding="utf-8"),
            )

    def test_expected_nonzero_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory(dir=TOOLING / "tests") as temporary:
            result, stdout, _, _ = self.run_watchdog(
                Path(temporary),
                [sys.executable, str(FIXTURE), "exit", "--exit-code", "7"],
                "--expected-exit-code",
                "7",
            )

            self.assertEqual(result.returncode, 0)
            self.assertEqual(stdout.read_bytes(), b"declared exit 7\n")

    def test_timeout_includes_only_bounded_cleanup_allowance(self) -> None:
        with tempfile.TemporaryDirectory(dir=TOOLING / "tests") as temporary:
            started = time.monotonic()
            result, stdout, _, log = self.run_watchdog(
                Path(temporary),
                [sys.executable, str(FIXTURE), "spawn-wait"],
                outer_timeout=5,
            )
            elapsed = time.monotonic() - started

            self.assertEqual(result.returncode, 124)
            self.assertLess(elapsed, 3.5)
            self.assertTrue(stdout.read_bytes().startswith(b"grandchild_pid="))
            self.assertIn(
                "process termination and output draining confirmed",
                log.read_text(encoding="utf-8"),
            )


if __name__ == "__main__":
    unittest.main()
