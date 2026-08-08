#!/usr/bin/env python3
"""Prepare the pinned Weston source tree for the GPUI Wayland client test."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tarfile

WESTON_VERSION = "16.0.0"
WESTON_COMMIT = "d1882b0a544ae2197b597a6e39478e719bc54302"
WESTON_TARBALL_SHA256 = (
    "dfb32e2bccabda957b94a8d0ec6075acd18c71c87ebc543ee3e618d294ca0f7f"
)
TEST_NAME = "gpui-wayland-input"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--tarball", type=Path, required=True)
    parser.add_argument("--source-parent", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--reader", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def prepare_source(
    tarball: Path,
    source_parent: Path,
    fixture: Path,
    reader: Path,
    metadata: Path,
) -> Path:
    actual_sha256 = sha256(tarball)
    if actual_sha256 != WESTON_TARBALL_SHA256:
        raise ValueError(
            f"Weston tarball SHA-256 was {actual_sha256}, expected {WESTON_TARBALL_SHA256}"
        )

    source_parent.mkdir(parents=True, exist_ok=True)
    source_dir = source_parent / f"weston-{WESTON_VERSION}"
    if source_dir.exists():
        raise FileExistsError(f"Weston source directory already exists: {source_dir}")

    with tarfile.open(tarball, "r:xz") as archive:
        roots = {Path(member.name).parts[0] for member in archive.getmembers() if member.name}
        expected_root = f"weston-{WESTON_VERSION}"
        if roots != {expected_root}:
            raise ValueError(f"Weston tarball roots were {sorted(roots)}, expected {expected_root}")
        archive.extractall(source_parent, filter="data")

    fixture_destination = source_dir / "tests" / f"{TEST_NAME}-test.c"
    shutil.copyfile(fixture, fixture_destination)
    reader_name = "gpui-wayland-clipboard-reader"
    reader_destination = source_dir / "tests" / f"{reader_name}.c"
    shutil.copyfile(reader, reader_destination)

    test_plugin_path = source_dir / "tests" / "harness" / "weston-test.c"
    test_plugin = test_plugin_path.read_text(encoding="utf-8")
    destroy_single_client = """\tassert(tsd->wl_client);
\ttsd->wl_client = NULL;
"""
    destroy_multiple_clients = """\tif (tsd->wl_client == wl_resource_get_client(resource))
\t\ttsd->wl_client = NULL;
"""
    bind_single_client = """\t/* There can only be one wl_client bound */
\tassert(!tsd->wl_client);
\ttsd->wl_client = client;
"""
    bind_multiple_clients = """\t/* Keep the first client as the harness owner while allowing the
\t * external clipboard reader to use activation requests. */
\tif (!tsd->wl_client)
\t\ttsd->wl_client = client;
"""
    if test_plugin.count(destroy_single_client) != 1:
        raise ValueError("could not locate the Weston test client destroy guard")
    if test_plugin.count(bind_single_client) != 1:
        raise ValueError("could not locate the Weston test client bind guard")
    test_plugin = test_plugin.replace(
        destroy_single_client,
        destroy_multiple_clients,
        1,
    ).replace(
        bind_single_client,
        bind_multiple_clients,
        1,
    )
    test_plugin_path.write_text(test_plugin, encoding="utf-8")

    meson_path = source_dir / "tests" / "meson.build"
    meson = meson_path.read_text(encoding="utf-8")
    marker = "\n]\n\nif get_option('renderer-gl')"
    if meson.count(marker) != 1:
        raise ValueError("could not locate the Weston tests list terminator")
    meson = meson.replace(
        marker,
        f"\n\t{{\t'name': '{TEST_NAME}', }},\n]\n\nif get_option('renderer-gl')",
        1,
    )
    reader_marker = "\nforeach t : tests\n"
    if meson.count(reader_marker) != 1:
        raise ValueError("could not locate the Weston test build loop")
    meson = meson.replace(
        reader_marker,
        f"""
gpui_wayland_clipboard_reader = executable(
\t'{reader_name}',
\t[
\t\t'{reader_name}.c',
\t\tweston_test_client_protocol_h,
\t\tweston_test_protocol_c,
\t],
\tbuild_by_default: true,
\tinclude_directories: common_inc,
\tdependencies: dep_wayland_client,
\tinstall: false,
)

foreach t : tests
""",
        1,
    )
    meson_path.write_text(meson, encoding="utf-8")

    metadata.parent.mkdir(parents=True, exist_ok=True)
    metadata.write_text(
        json.dumps(
            {
                "weston_version": WESTON_VERSION,
                "weston_commit": WESTON_COMMIT,
                "weston_tarball_sha256": WESTON_TARBALL_SHA256,
                "test_name": TEST_NAME,
                "clipboard_reader": reader_name,
                "weston_test_multi_client_patch": True,
                "backend": "headless",
                "renderer": "pixman",
                "shell": "test-desktop",
                "output": {
                    "width": 320,
                    "height": 240,
                    "transform": "normal",
                    "refresh_millihertz": 60000,
                },
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return source_dir


def main() -> int:
    args = parse_args()
    source_dir = prepare_source(
        args.tarball,
        args.source_parent,
        args.fixture,
        args.reader,
        args.metadata,
    )
    print(source_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
