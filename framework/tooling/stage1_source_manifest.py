#!/usr/bin/env python3
"""Record and verify exact source identity for retained Stage 1 evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
from typing import Any

import stage1_process


SCHEMA_VERSION = 1
IDENTITY_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "compatibility.toml",
    ".github/workflows/stage1.yml",
)


class SourceIdentityError(RuntimeError):
    pass


def git(*arguments: str) -> str:
    result = stage1_process.run_capture(
        ["git", *arguments],
        timeout_seconds=30,
    )
    if result.timed_out:
        raise SourceIdentityError(f"git {' '.join(arguments)} timed out")
    if result.cleanup_timed_out:
        raise SourceIdentityError(f"git {' '.join(arguments)} cleanup was not confirmed")
    if result.returncode != 0:
        stderr = result.stderr.decode("utf-8", errors="replace").strip()
        raise SourceIdentityError(f"git {' '.join(arguments)} failed: {stderr}")
    return result.stdout.decode("utf-8").strip()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_snapshot() -> dict[str, Any]:
    repository_root = Path(git("rev-parse", "--show-toplevel"))
    status = git("status", "--porcelain=v1", "--untracked-files=all")
    if status:
        raise SourceIdentityError(f"source checkout is not clean:\n{status}")

    files: dict[str, str] = {}
    for relative_path in IDENTITY_FILES:
        path = repository_root / relative_path
        if not path.is_file():
            raise SourceIdentityError(f"identity file is missing: {relative_path}")
        files[relative_path] = file_sha256(path)

    return {
        "head_commit": git("rev-parse", "HEAD"),
        "head_tree": git("rev-parse", "HEAD^{tree}"),
        "identity_file_sha256": files,
        "source_clean": True,
    }


def workflow_identity() -> dict[str, str]:
    names = (
        "GITHUB_REPOSITORY",
        "GITHUB_SHA",
        "GITHUB_REF",
        "GITHUB_EVENT_NAME",
        "GITHUB_WORKFLOW",
        "GITHUB_JOB",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
    )
    return {name.lower(): os.environ.get(name, "") for name in names}


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def record(artifact_dir: Path) -> None:
    snapshot = source_snapshot()
    workflow = workflow_identity()
    expected_sha = workflow["github_sha"]
    if expected_sha and snapshot["head_commit"] != expected_sha:
        raise SourceIdentityError(
            f"checked-out commit {snapshot['head_commit']} does not match GITHUB_SHA {expected_sha}"
        )
    write_json(
        artifact_dir / "source-manifest.json",
        {
            "schema_version": SCHEMA_VERSION,
            "source": snapshot,
            "workflow": workflow,
        },
    )


def verify(artifact_dir: Path) -> None:
    manifest_path = artifact_dir / "source-manifest.json"
    verification_path = artifact_dir / "source-verification.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("schema_version") != SCHEMA_VERSION:
            raise SourceIdentityError("source manifest schema is missing or unsupported")
        expected_source = manifest.get("source")
        actual_source = source_snapshot()
        if actual_source != expected_source:
            raise SourceIdentityError("source identity changed after evidence execution")
        expected_workflow = manifest.get("workflow")
        actual_workflow = workflow_identity()
        if actual_workflow != expected_workflow:
            raise SourceIdentityError("workflow identity changed after evidence execution")
        write_json(
            verification_path,
            {
                "outcome": "passed",
                "schema_version": SCHEMA_VERSION,
                "source": actual_source,
                "workflow": actual_workflow,
            },
        )
    except (OSError, ValueError, SourceIdentityError) as error:
        write_json(
            verification_path,
            {
                "error": str(error),
                "outcome": "failed",
                "schema_version": SCHEMA_VERSION,
            },
        )
        raise SourceIdentityError(str(error)) from error


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("record", "verify"))
    parser.add_argument("--artifact-dir", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.mode == "record":
            record(args.artifact_dir)
        else:
            verify(args.artifact_dir)
    except SourceIdentityError as error:
        print(f"source identity error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
