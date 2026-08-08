---
title: "Local GPUI Override Workflow"
summary: "Use an uncommitted Cargo patch for coordinated framework and GPUI development."
read_when: "testing framework changes against a sibling GPUI checkout or changing workspace GPUI dependency pins"
---
# Local GPUI Override Workflow

The committed framework always uses the immutable GPUI Git revision in the
workspace manifest. The repository does not vendor GPUI and has no GPUI
submodule.

For coordinated development, create an untracked disposable framework snapshot
in `/tmp`. Copy only source files, excluding Git metadata and build or docs
output. Add the override only to the snapshot's `.cargo/config.toml`; never edit
the tracked checkout or global `$CARGO_HOME/config.toml`:

```bash
snapshot=$(mktemp -d /tmp/gpui-component-gpui-override.XXXXXX)
rsync -a --exclude='.git' --exclude='target' --exclude='docs/node_modules' ./ "$snapshot/"
cd "$snapshot"
```

Replace `/absolute/path/to/gpui` below with the absolute path of the sibling
GPUI checkout. Absolute paths are required because the framework snapshot lives
outside the sibling checkout directory.

```toml
[patch."https://github.com/BumpyClock/gpui"]
bumpyclock-gpui = { path = "/absolute/path/to/gpui/crates/gpui" }
gpui_platform = { path = "/absolute/path/to/gpui/crates/gpui_platform" }
gpui_macros = { path = "/absolute/path/to/gpui/crates/gpui_macros" }
sum_tree = { path = "/absolute/path/to/gpui/crates/sum_tree" }
```

The copied `Cargo.lock` still records Git sources, so first refresh it only in
the disposable snapshot. Subsequent validation remains locked:

```bash
cargo metadata --format-version 1
cargo check --locked --workspace --all-targets
cargo xtask release-check
```

Patch keys are Cargo package identities, not dependency aliases or Rust import
names. The framework manifest must already declare
`gpui = { package = "bumpyclock-gpui", ... }`; a patch cannot bridge an old
`gpui` package identity to the renamed package. During an identity transition,
test in a disposable framework snapshot and wait for the canonical GPUI commit
before changing the committed pin.

Add every GPUI package resolved by the framework to the patch table. Because
the snapshot has no Git metadata, this developer-specific override cannot enter
a commit. Do not use Cargo's `--config` flag for this workflow: `cargo xtask`
launches child Cargo processes that do not inherit the parent command line's
config. Before release work, discard the disposable snapshot, return to the
tracked checkout, and verify the committed Git dependency:

```bash
cargo xtask compatibility check
cargo xtask release-check
```

After the GPUI change merges, update the framework's full GPUI revision and
exact registry versions together, regenerate compatibility documentation, then
repeat the checks above. Engine packages must be published before framework
packages; the committed override never represents a release dependency.
