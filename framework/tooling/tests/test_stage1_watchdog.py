from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


TOOLING = Path(__file__).parents[1]
WATCHDOG = TOOLING / "stage1_watchdog.py"
FIXTURE = Path(__file__).parent / "fixtures" / "process_tree.py"


class Stage1WatchdogTests(unittest.TestCase):
    def run_watchdog(
        self,
        directory: Path,
        command: list[str],
        *options: str,
        outer_timeout: float = 10,
    ) -> tuple[subprocess.CompletedProcess[bytes], Path, Path, Path]:
        stdout = directory / "stdout.log"
        stderr = directory / "stderr.log"
        log = directory / "watchdog.log"
        result = subprocess.run(
            [
                sys.executable,
                str(WATCHDOG),
                "--timeout-seconds",
                "2",
                "--cleanup-seconds",
                "1",
                "--stdout",
                str(stdout),
                "--stderr",
                str(stderr),
                "--log",
                str(log),
                *options,
                "--",
                *command,
            ],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=outer_timeout,
        )
        return result, stdout, stderr, log

    def test_success_preserves_complete_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, stdout, stderr, _ = self.run_watchdog(
                Path(temporary), [sys.executable, str(FIXTURE), "success"]
            )

            self.assertEqual(result.returncode, 0)
            self.assertEqual(stdout.read_bytes(), b"stage1 stdout complete\n")
            self.assertEqual(stderr.read_bytes(), b"stage1 stderr complete\n")

    def test_expected_nonzero_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result, stdout, _, _ = self.run_watchdog(
                Path(temporary),
                [sys.executable, str(FIXTURE), "exit", "--exit-code", "7"],
                "--expected-exit-code",
                "7",
            )

            self.assertEqual(result.returncode, 0)
            self.assertEqual(stdout.read_bytes(), b"declared exit 7\n")

    def test_timeout_includes_only_bounded_cleanup_allowance(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            started = time.monotonic()
            result, _, _, log = self.run_watchdog(
                Path(temporary),
                [sys.executable, str(FIXTURE), "spawn-wait"],
                outer_timeout=5,
            )
            elapsed = time.monotonic() - started

            self.assertEqual(result.returncode, 124)
            self.assertLess(elapsed, 3.5)
            self.assertIn(
                "process termination and output draining confirmed",
                log.read_text(encoding="utf-8"),
            )


if __name__ == "__main__":
    unittest.main()
