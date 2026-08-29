from __future__ import annotations

import ast
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parents[1]))
import stage1_source_manifest  # noqa: E402


SCRIPT = Path(__file__).parents[1] / "stage1_source_manifest.py"
REPOSITORY_ROOT = Path(__file__).parents[3]
STAGE1_WORKFLOW = REPOSITORY_ROOT / ".github/workflows/stage1.yml"
STAGE1_CONTRACT = REPOSITORY_ROOT / "framework/STAGE1-CONTRACT.md"
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
        for relative_path in stage1_source_manifest.IDENTITY_FILES:
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
            "GITHUB_REPOSITORY": "BumpyClock/neutron",
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
        self.assertEqual(
            set(manifest["source"]["identity_file_sha256"]),
            set(stage1_source_manifest.IDENTITY_FILES),
        )
        self.assertEqual(manifest["workflow"]["github_repository"], "BumpyClock/neutron")
        self.assertTrue(manifest["source"]["source_clean"])
        self.assertEqual(verification["outcome"], "passed")
        self.assertFalse((root / "tooling/__pycache__").exists())

    def test_record_rejects_dirty_source(self) -> None:
        _, root, commit = self.repository()
        (root / "Cargo.toml").write_text("changed\n", encoding="utf-8")

        result = self.run_script(root, "record", commit)

        self.assertEqual(result.returncode, 1)
        self.assertIn("source checkout is not clean", result.stderr)

    def test_record_hashes_committed_blobs_when_checkout_line_endings_differ(self) -> None:
        _, root, commit = self.repository()
        subprocess.run(
            ["git", "config", "core.autocrlf", "true"],
            cwd=root,
            check=True,
            timeout=5,
        )
        cargo_toml = root / "Cargo.toml"
        cargo_toml.unlink()
        subprocess.run(
            ["git", "checkout", "--", "Cargo.toml"],
            cwd=root,
            check=True,
            timeout=5,
        )
        self.assertIn(b"\r\n", cargo_toml.read_bytes())
        status = subprocess.run(
            ["git", "status", "--porcelain=v1"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            text=True,
            timeout=5,
        ).stdout
        self.assertEqual(status, "")

        result = self.run_script(root, "record", commit)

        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads((root / "artifacts/source-manifest.json").read_text())
        committed = subprocess.run(
            ["git", "show", f"{commit}:Cargo.toml"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            timeout=5,
        ).stdout
        self.assertEqual(
            manifest["source"]["identity_file_sha256"]["Cargo.toml"],
            hashlib.sha256(committed).hexdigest(),
        )

    def test_verify_retains_failure_when_source_changes(self) -> None:
        _, root, commit = self.repository()
        self.assertEqual(self.run_script(root, "record", commit).returncode, 0)
        (root / "Cargo.lock").write_text("changed\n", encoding="utf-8")

        result = self.run_script(root, "verify", commit)

        self.assertEqual(result.returncode, 1)
        verification = json.loads((root / "artifacts/source-verification.json").read_text())
        self.assertEqual(verification["outcome"], "failed")
        self.assertIn("source checkout is not clean", verification["error"])


class Stage1IdentityParityTests(unittest.TestCase):
    """Parity checks against the real repository's stage1.yml and contract.

    These read the actual committed files rather than fixtures so the
    canonical identity list cannot silently drift between the workflow
    aggregate, the contract documentation, and IDENTITY_FILES.
    """

    def test_workflow_aggregate_matches_identity_files(self) -> None:
        text = STAGE1_WORKFLOW.read_text(encoding="utf-8")
        match = re.search(r"identity_paths = \((.*?)\)\n", text, re.DOTALL)
        self.assertIsNotNone(match, "identity_paths tuple not found in stage1.yml")
        paths = ast.literal_eval("(" + match.group(1) + ")")
        self.assertEqual(paths, stage1_source_manifest.IDENTITY_FILES)

    def test_contract_canonical_list_matches_identity_files(self) -> None:
        text = STAGE1_CONTRACT.read_text(encoding="utf-8")
        paths = tuple(re.findall(r"^\d+\. `([^`]+)`$", text, re.MULTILINE))
        self.assertEqual(paths, stage1_source_manifest.IDENTITY_FILES)


if __name__ == "__main__":
    unittest.main()
