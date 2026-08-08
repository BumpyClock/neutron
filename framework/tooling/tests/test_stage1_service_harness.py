from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
HARNESS = REPOSITORY_ROOT / "tooling" / "stage1_service_harness.py"


@unittest.skipUnless(sys.platform == "linux", "service harness owns Linux process groups")
class ServiceHarnessTests(unittest.TestCase):
    def run_harness(
        self,
        service_code: str,
        payload_code: str,
        *,
        timeout_seconds: float = 2.0,
    ) -> tuple[subprocess.CompletedProcess[str], Path]:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        directory = Path(temporary_directory.name)
        command = [
            sys.executable,
            str(HARNESS),
            "--timeout-seconds",
            str(timeout_seconds),
            "--service-executable",
            sys.executable,
            "--service-argument=-c",
            f"--service-argument={service_code}",
            "--service-stdout",
            str(directory / "service.stdout"),
            "--service-stderr",
            str(directory / "service.stderr"),
            "--payload-stdout",
            str(directory / "payload.stdout"),
            "--payload-stderr",
            str(directory / "payload.stderr"),
            "--log",
            str(directory / "supervisor.log"),
            "--",
            sys.executable,
            "-c",
            payload_code,
        ]
        result = subprocess.run(command, capture_output=True, encoding="utf-8", timeout=8)
        return result, directory

    def test_success_cleans_long_running_service(self) -> None:
        result, directory = self.run_harness(
            "import time; time.sleep(30)",
            "print('payload passed')",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (directory / "payload.stdout").read_text(encoding="utf-8"),
            "payload passed\n",
        )
        self.assertIn(
            "cleanup_confirmed=true",
            (directory / "supervisor.log").read_text(encoding="utf-8"),
        )

    def test_simultaneous_exit_prefers_service_failure(self) -> None:
        result, directory = self.run_harness(
            "pass",
            "pass",
        )

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(
            "outcome=service_exited",
            (directory / "supervisor.log").read_text(encoding="utf-8"),
        )

    def test_early_service_exit_cleans_payload_and_descendant(self) -> None:
        with tempfile.TemporaryDirectory() as marker_directory:
            marker = Path(marker_directory) / "descendant.json"
            service_code = (
                "import json,pathlib,subprocess,sys; "
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
                f"pathlib.Path({str(marker)!r}).write_text(json.dumps({{'pid': child.pid}}))"
            )
            result, directory = self.run_harness(
                service_code,
                "import time; time.sleep(30)",
            )
            descendant_pid = json.loads(marker.read_text(encoding="utf-8"))["pid"]

            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(
                "outcome=service_exited",
                (directory / "supervisor.log").read_text(encoding="utf-8"),
            )
            self.assertIn(
                "cleanup_confirmed=true",
                (directory / "supervisor.log").read_text(encoding="utf-8"),
            )
            for _ in range(50):
                try:
                    os.kill(descendant_pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.02)
            else:
                state = Path(f"/proc/{descendant_pid}/stat").read_text(encoding="utf-8").split()[2]
                self.assertEqual(state, "Z", "service descendant remained live after cleanup")

    def test_early_service_exit_cleans_nested_managed_group(self) -> None:
        with tempfile.TemporaryDirectory() as marker_directory:
            marker = Path(marker_directory) / "nested.pid"
            child_code = (
                "import pathlib,os,time; "
                f"pathlib.Path({str(marker)!r}).write_text(str(os.getpid())); "
                "time.sleep(30)"
            )
            payload_code = (
                "import os,sys; "
                f"sys.path.insert(0, {str(HARNESS.parent)!r}); "
                "import stage1_process; "
                f"stage1_process.run_capture([sys.executable,'-c',{child_code!r}], "
                "timeout_seconds=30, environment={'PATH': os.environ.get('PATH', '')})"
            )
            result, directory = self.run_harness(
                "import time; time.sleep(0.2)",
                payload_code,
            )
            nested_pid = int(marker.read_text(encoding="utf-8"))

            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(
                "cleanup_confirmed=true",
                (directory / "supervisor.log").read_text(encoding="utf-8"),
            )
            for _ in range(50):
                try:
                    os.kill(nested_pid, 0)
                except ProcessLookupError:
                    break
                stat = Path(f"/proc/{nested_pid}/stat").read_text(encoding="utf-8")
                if stat[stat.rfind(")") + 2 :].startswith("Z "):
                    break
                time.sleep(0.02)
            else:
                self.fail("nested managed command remained live after service exit")

    def test_timeout_is_124_with_confirmed_shared_cleanup(self) -> None:
        result, directory = self.run_harness(
            "import time; time.sleep(30)",
            "import time; time.sleep(30)",
            timeout_seconds=0.1,
        )

        self.assertEqual(result.returncode, 124, result.stderr)
        log = (directory / "supervisor.log").read_text(encoding="utf-8")
        self.assertIn("outcome=timed_out", log)
        self.assertIn("cleanup_confirmed=true", log)


if __name__ == "__main__":
    unittest.main()
