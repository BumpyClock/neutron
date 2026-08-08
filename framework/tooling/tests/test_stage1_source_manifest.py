from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parents[1] / "stage1_source_manifest.py"
GITHUB_VARIABLES = (
    "GITHUB_REPOSITORY",
    "GITHUB_SHA",
    "GITHUB_REF",
    "GITHUB_EVENT_NAME",
    "GITHUB_WORKFLOW",
    "GITHUB_JOB",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
)


class Stage1SourceManifestTests(unittest.TestCase):
    def repository(self) -> tuple[tempfile.TemporaryDirectory[str], Path, str]:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        for relative_path in (
            "Cargo.toml",
            "Cargo.lock",
            "compatibility.toml",
            ".github/workflows/stage1.yml",
        ):
            path = root / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"identity: {relative_path}\n", encoding="utf-8")
        tooling = root / "tooling"
        tooling.mkdir()
        shutil.copy2(SCRIPT, tooling / SCRIPT.name)
        shutil.copy2(SCRIPT.parent / "stage1_process.py", tooling / "stage1_process.py")
        subprocess.run(["git", "init", "-q"], cwd=root, check=True, timeout=5)
        (root / ".git/info/exclude").write_text("artifacts/\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=root, check=True, timeout=5)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Stage 1 Test",
                "-c",
                "user.email=stage1@example.invalid",
                "commit",
                "-qm",
                "test source",
            ],
            cwd=root,
            check=True,
            timeout=5,
        )
        commit = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            text=True,
            timeout=5,
        ).stdout.strip()
        return temporary, root, commit

    def run_script(
        self,
        root: Path,
        mode: str,
        commit: str,
    ) -> subprocess.CompletedProcess[str]:
        environment = {key: value for key, value in os.environ.items() if key not in GITHUB_VARIABLES}
        environment |= {
            "PYTHONDONTWRITEBYTECODE": "1",
            "GITHUB_REPOSITORY": "BumpyClock/gpui-component",
            "GITHUB_SHA": commit,
            "GITHUB_REF": "refs/heads/test",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_WORKFLOW": "Stage 1 runtime evidence",
            "GITHUB_JOB": "source-test",
            "GITHUB_RUN_ID": "1",
            "GITHUB_RUN_ATTEMPT": "1",
        }
        return subprocess.run(
            [
                sys.executable,
                str(root / "tooling" / SCRIPT.name),
                mode,
                "--artifact-dir",
                str(root / "artifacts"),
            ],
            cwd=root,
            env=environment,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=15,
        )

    def test_record_and_verify_exact_clean_source(self) -> None:
        _, root, commit = self.repository()

        record = self.run_script(root, "record", commit)
        verify = self.run_script(root, "verify", commit)

        self.assertEqual(record.returncode, 0, record.stderr)
        self.assertEqual(verify.returncode, 0, verify.stderr)
        manifest = json.loads((root / "artifacts/source-manifest.json").read_text())
        verification = json.loads((root / "artifacts/source-verification.json").read_text())
        self.assertEqual(manifest["source"]["head_commit"], commit)
        self.assertTrue(manifest["source"]["source_clean"])
        self.assertEqual(verification["outcome"], "passed")
        self.assertFalse((root / "tooling/__pycache__").exists())

    def test_record_rejects_dirty_source(self) -> None:
        _, root, commit = self.repository()
        (root / "Cargo.toml").write_text("changed\n", encoding="utf-8")

        result = self.run_script(root, "record", commit)

        self.assertEqual(result.returncode, 1)
        self.assertIn("source checkout is not clean", result.stderr)

    def test_verify_retains_failure_when_source_changes(self) -> None:
        _, root, commit = self.repository()
        self.assertEqual(self.run_script(root, "record", commit).returncode, 0)
        (root / "Cargo.lock").write_text("changed\n", encoding="utf-8")

        result = self.run_script(root, "verify", commit)

        self.assertEqual(result.returncode, 1)
        verification = json.loads((root / "artifacts/source-verification.json").read_text())
        self.assertEqual(verification["outcome"], "failed")
        self.assertIn("source checkout is not clean", verification["error"])


if __name__ == "__main__":
    unittest.main()
