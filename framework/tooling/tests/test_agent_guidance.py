from __future__ import annotations

import re
from pathlib import Path
import unittest


FRAMEWORK = Path(__file__).resolve().parents[2]
CANONICAL = FRAMEWORK / ".agents" / "skills"
ADAPTERS = FRAMEWORK / ".claude" / "skills"
LINK = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
FRONTMATTER = re.compile(r"\A---\n(.*?)\n---\n", re.DOTALL)


class AgentGuidanceTests(unittest.TestCase):
    def test_local_guidance_links_resolve(self) -> None:
        files = [
            *CANONICAL.rglob("*.md"),
            *ADAPTERS.rglob("*.md"),
            FRAMEWORK / ".claude" / "COMPONENT_TEST_RULES.md",
        ]
        for path in files:
            for target in LINK.findall(path.read_text(encoding="utf-8")):
                if "://" in target or target.startswith("#"):
                    continue
                with self.subTest(path=path, target=target):
                    self.assertTrue(
                        (path.parent / target.split("#", 1)[0]).exists(),
                        f"Missing local link: {target}",
                    )

    def test_claude_adapters_route_to_canonical_files_with_matching_metadata(self) -> None:
        adapters = list(ADAPTERS.rglob("*.md"))
        self.assertTrue(adapters, "Claude skill entrypoints are missing")
        for path in adapters:
            canonical = CANONICAL / path.relative_to(ADAPTERS)
            with self.subTest(path=path):
                self.assertTrue(canonical.is_file())
                self.assertFalse(path.is_symlink())
                self.assertNotEqual(path.stat().st_ino, canonical.stat().st_ino)
                content = path.read_text(encoding="utf-8")
                links = LINK.findall(content)
                self.assertEqual(len(links), 1, "Adapters must route directly to one canonical file")
                self.assertEqual((path.parent / links[0]).resolve(), canonical.resolve())
                if path.name == "SKILL.md":
                    adapter_metadata = FRONTMATTER.match(content)
                    canonical_metadata = FRONTMATTER.match(canonical.read_text(encoding="utf-8"))
                    self.assertIsNotNone(adapter_metadata)
                    self.assertIsNotNone(canonical_metadata)
                    self.assertEqual(adapter_metadata[1], canonical_metadata[1])


if __name__ == "__main__":
    unittest.main()
