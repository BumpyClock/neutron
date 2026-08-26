from __future__ import annotations

import re
from pathlib import Path
import unittest


ROOT = Path(__file__).parents[3]
DOCS_ROOT = ROOT / "framework" / "docs" / "docs" / "components"
CATALOG = DOCS_ROOT / "index.md"
STORY_REGISTRY = ROOT / "framework" / "crates" / "story" / "src" / "lib.rs"

CATALOG_LINK = re.compile(r"^\s*-\s*\[([^\]]+)\]\(([^)#]+)(?:#[^)]+)?\)", re.MULTILINE)
STORY_DESCRIPTOR = re.compile(
    r'story_descriptor!\(\s*"[^"]+"\s*,\s*([A-Za-z][A-Za-z0-9_]*)\s*,\s*"([^"]+)"\s*\)'
)

REQUIRED_COMPONENTS = {
    "Breadcrumb": ("breadcrumb", "BreadcrumbStory"),
    "Command Palette": ("command-palette", "CommandPaletteStory"),
    "Divider": ("divider", "DividerStory"),
}

# These catalog pages are valid before gallery stories exist.
CATALOG_ONLY_COMPONENTS = frozenset({"Editor", "Scrollable"})


def catalog_links(markdown: str) -> dict[str, str]:
    matches = CATALOG_LINK.findall(markdown)
    links = {
        label: target.removeprefix("./").removesuffix(".md").removesuffix("/")
        for label, target in matches
    }
    if len(links) != len(matches):
        raise ValueError("docs catalog contains duplicate component labels")
    return links


def story_descriptors(source: str) -> set[str]:
    return {story_klass for _, story_klass in STORY_DESCRIPTOR.findall(source)}


def story_klass_for_label(label: str) -> str:
    normalized_label = re.sub(r"[^A-Za-z0-9]", "", label)
    return f"{normalized_label}Story"


class ComponentCatalogTests(unittest.TestCase):
    def test_catalog_entries_have_pages_and_gallery_descriptors(self) -> None:
        links = catalog_links(CATALOG.read_text(encoding="utf-8"))
        descriptors = story_descriptors(STORY_REGISTRY.read_text(encoding="utf-8"))

        self.assertSetEqual(
            set(REQUIRED_COMPONENTS) - set(links),
            set(),
            "required components are missing from the docs catalog",
        )
        self.assertDictEqual(
            {
                label: links[label]
                for label, (target, _) in REQUIRED_COMPONENTS.items()
                if links.get(label) != target
            },
            {},
            "required components use unexpected docs paths",
        )

        missing_pages = {
            label: target
            for label, target in links.items()
            if not (DOCS_ROOT / f"{target}.md").is_file()
        }
        self.assertDictEqual(
            missing_pages,
            {},
            "catalog entries must resolve to documentation pages",
        )

        catalog_entries_without_stories = {
            label
            for label in links
            if story_klass_for_label(label) not in descriptors
        }
        self.assertSetEqual(
            catalog_entries_without_stories,
            set(CATALOG_ONLY_COMPONENTS),
            "catalog entries without gallery descriptors must use named exemptions",
        )
        self.assertDictEqual(
            {
                label: story_klass_for_label(label)
                for label in links
                if label not in CATALOG_ONLY_COMPONENTS
                and story_klass_for_label(label) not in descriptors
            },
            {},
            "non-exempt catalog entries are missing from the story registry",
        )


if __name__ == "__main__":
    unittest.main()
