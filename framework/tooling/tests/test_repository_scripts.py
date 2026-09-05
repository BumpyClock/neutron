from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parents[3]
BASH = shutil.which("bash")
COMMAND_STUBS = r"""
cargo() {
    printf 'cargo' >> "$COMMAND_LOG"
    printf '\t%s' "$@" >> "$COMMAND_LOG"
    printf '\n' >> "$COMMAND_LOG"
    if [[ "$*" == *"${FAIL_COMMAND:-not-a-real-command}"* ]]; then
        printf '%s\n' "source test failed; root [patch.crates-io] publication blocker" >&2
        return 17
    fi
}
python3() {
    printf 'python3' >> "$COMMAND_LOG"
    printf '\t%s' "$@" >> "$COMMAND_LOG"
    printf '\n' >> "$COMMAND_LOG"
}
export -f cargo python3
"""
STUB_COMMANDS = COMMAND_STUBS + '\nbash "$@"\n'


@unittest.skipUnless(BASH, "requires Bash")
class RepositoryScriptTests(unittest.TestCase):
    def run_script(
        self, name: str, *arguments: str, fail_command: str = ""
    ) -> tuple[subprocess.CompletedProcess[str], list[list[str]]]:
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "commands.log"
            environment = os.environ | {
                "COMMAND_LOG": str(log),
                "FAIL_COMMAND": fail_command,
            }
            result = subprocess.run(
                [BASH, "-c", STUB_COMMANDS, "script-test", str(ROOT / "script" / name), *arguments],
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                timeout=10,
            )
            commands = [
                line.split("\t") for line in log.read_text().splitlines()
            ] if log.exists() else []
        return result, commands

    def test_help_runs_no_commands(self) -> None:
        for script in ("check", "test", "stage1", "release-check"):
            with self.subTest(script=script):
                result, commands = self.run_script(script, "--help")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Usage:", result.stdout)
                self.assertEqual(commands, [])

    def test_targeted_test_forwards_options_without_workspace_or_tooling(self) -> None:
        arguments = (
            "-p", "neutron-components-app", "--test", "headless",
            "--features", "test-support", "--", "--nocapture",
        )
        result, commands = self.run_script("test", *arguments)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(commands, [["cargo", "test", "--locked", *arguments]])

    def test_targeted_failure_is_not_masked(self) -> None:
        result, commands = self.run_script(
            "test", "-p", "neutron-components-manifest", fail_command="test --locked"
        )
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(len(commands), 1)

    def test_empty_filter_is_forwarded_with_package_selection(self) -> None:
        result, commands = self.run_script("test", "", "-p", "engine-xtask")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(commands, [["cargo", "test", "--locked", "", "-p", "engine-xtask"]])

    def test_default_test_preserves_all_suites(self) -> None:
        result, commands = self.run_script("test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(commands[:2], [
            ["cargo", "test", "--locked", "--workspace", "--all-targets", "--features", "test-support"],
            ["cargo", "test", "--locked", "--workspace", "--doc"],
        ])
        self.assertEqual(commands[2:], [
            ["python3", "-m", "unittest", "discover", "-s", "framework/tooling", "-p", "test_*.py"],
            ["python3", "-m", "unittest", "discover", "-s", "framework/tooling/tests", "-p", "test_*.py"],
        ])

    def test_tooling_mode_skips_cargo(self) -> None:
        result, commands = self.run_script("test", "--tooling")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(commands), 2)
        self.assertTrue(all(command[0] == "python3" for command in commands))

    def test_package_check_retains_both_domain_contracts(self) -> None:
        result, commands = self.run_script(
            "check", "-p", "framework-xtask", "--package", "engine-xtask"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        selection = ["-p", "framework-xtask", "-p", "engine-xtask"]
        self.assertIn(["cargo", "check", "--locked", *selection, "--all-targets"], commands)
        self.assertIn([
            "cargo", "clippy", "--locked", *selection,
            "--all-targets", "--all-features", "--", "--deny", "warnings",
        ], commands)
        self.assertEqual(commands[-2:], [
            ["cargo", "run", "--locked", "-p", "engine-xtask", "--", "fork", "validate"],
            ["cargo", "run", "--locked", "-p", "framework-xtask", "--", "compatibility", "check"],
        ])

    def test_default_check_selects_workspace(self) -> None:
        result, commands = self.run_script("check")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            ["cargo", "check", "--locked", "--workspace", "--all-targets"], commands
        )

    def test_invalid_script_arguments_fail_before_commands(self) -> None:
        for script, arguments in (
            ("check", ("-p",)),
            ("check", ("--unknown",)),
            ("test", ("--tooling", "-p", "framework-xtask")),
            ("stage1", ("-p", "framework-xtask")),
            ("release-check", ("--unknown",)),
        ):
            with self.subTest(script=script, arguments=arguments):
                result, commands = self.run_script(script, *arguments)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(commands, [])

    def test_release_failure_still_runs_framework_and_remains_failure(self) -> None:
        result, commands = self.run_script(
            "release-check", fail_command="engine-xtask -- release-check"
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(len(commands), 6)
        self.assertEqual(commands[-1], [
            "cargo", "run", "--locked", "-p", "framework-xtask", "--", "release-check",
        ])
        self.assertIn("root [patch.crates-io]", result.stderr)

    def test_strict_release_option_reaches_both_domains(self) -> None:
        result, commands = self.run_script("release-check", "--require-registry")
        self.assertEqual(result.returncode, 0, result.stderr)
        for package in ("engine-xtask", "framework-xtask"):
            self.assertIn([
                "cargo", "run", "--locked", "-p", package,
                "--", "release-check", "--require-registry",
            ], commands)

    def test_release_workflow_preserves_failure_with_known_blocker_text(self) -> None:
        workflow = (ROOT / ".github/workflows/release-validation.yml").read_text()
        run_block = workflow.split("        run: |\n", 1)[1]
        commands = []
        for line in run_block.splitlines():
            if line.strip() and not line.startswith("          "):
                break
            commands.append(line[10:])
        self.assertTrue(commands)

        for failure in ("", "engine-xtask -- release-check", "framework-xtask -- release-check"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "script").mkdir()
                shutil.copy2(ROOT / "script/release-check", root / "script/release-check")
                result = subprocess.run(
                    [BASH, "-c", COMMAND_STUBS + "\n" + "\n".join(commands)],
                    cwd=root,
                    env=os.environ | {
                        "COMMAND_LOG": str(root / "commands.log"),
                        "FAIL_COMMAND": failure,
                    },
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                self.assertEqual(result.returncode, 1 if failure else 0, result.stderr)
                self.assertTrue((root / "release-validation.log").is_file())
                if failure:
                    self.assertIn("root [patch.crates-io]", result.stdout)


if __name__ == "__main__":
    unittest.main()
