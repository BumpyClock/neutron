# Neutron Migration Record

This record captures the immutable source baselines for the Neutron consolidation. The source repositories remain separate and unchanged.

## Source Baselines

| Domain | Repository | Local source | Branch | Commit | Tree | Status | Tags | Package family | Declared Rust |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Engine | `https://github.com/BumpyClock/gpui` | `/Users/adityasharma/Projects/rusty/gpui` | `main` | `198635ca8435c38dd4dfe796089145cdbefae36c` | `fa9bd19f07fdcc534753457ca109462b49e8930f` | clean | none | `0.1.0` | `1.95.0` |
| Framework | `https://github.com/BumpyClock/gpui-component` | `/Users/adityasharma/Projects/gpui-component` | `main` | `a3ff1b3132f1b12aee463dd94ece22271b98fd72` | `4bc1539532055e69abea2b9740d72b3d262063f8` | clean | `v0.6.0` | `0.7.0` | `1.90` (effective `1.95.0` with engine) |

## Immutable Evidence

The following commands were run in each source worktree. Empty `git status` output means no tracked or untracked changes.

### Engine

```text
$ git -C /Users/adityasharma/Projects/rusty/gpui status --porcelain=v1 --untracked-files=all
(empty)
$ git -C /Users/adityasharma/Projects/rusty/gpui rev-parse HEAD HEAD^{tree}
198635ca8435c38dd4dfe796089145cdbefae36c
fa9bd19f07fdcc534753457ca109462b49e8930f
$ git -C /Users/adityasharma/Projects/rusty/gpui branch --show-current
main
$ git -C /Users/adityasharma/Projects/rusty/gpui remote -v
origin	https://github.com/BumpyClock/gpui.git (fetch)
origin	https://github.com/BumpyClock/gpui.git (push)
$ git -C /Users/adityasharma/Projects/rusty/gpui tag --list
(empty)
```

### Framework

```text
$ git -C /Users/adityasharma/Projects/gpui-component status --porcelain=v1 --untracked-files=all
(empty)
$ git -C /Users/adityasharma/Projects/gpui-component rev-parse HEAD HEAD^{tree}
a3ff1b3132f1b12aee463dd94ece22271b98fd72
4bc1539532055e69abea2b9740d72b3d262063f8
$ git -C /Users/adityasharma/Projects/gpui-component branch --show-current
main
$ git -C /Users/adityasharma/Projects/gpui-component remote -v
origin	https://github.com/BumpyClock/gpui-component.git (fetch)
origin	https://github.com/BumpyClock/gpui-component.git (push)
$ git -C /Users/adityasharma/Projects/gpui-component tag --list
v0.6.0
```

## Import Method

Import each source as an exact committed snapshot with `git archive`:

```bash
git -C /Users/adityasharma/Projects/rusty/gpui archive --format=tar 198635ca8435c38dd4dfe796089145cdbefae36c | tar -xf - -C /Users/adityasharma/Projects/neutron/engine
git -C /Users/adityasharma/Projects/gpui-component archive --format=tar a3ff1b3132f1b12aee463dd94ece22271b98fd72 | tar -xf - -C /Users/adityasharma/Projects/neutron/framework
```

The migration imports committed files only. The selected source commits contain no tracked build output. The import excludes source `.git` metadata, histories, tags, untracked files, and build output. Do not import source commit histories or tags. Do not install `git-filter-repo` for this migration.

## Destination

- Local destination: `/Users/adityasharma/Projects/neutron`
- GitHub destination: `https://github.com/BumpyClock/neutron`
- Default branch: `main`

The target layout uses one root Git repository and one root Cargo workspace. Engine files belong under `engine/`; framework files belong under `framework/`.

## Destination Snapshot Facts

The destination imported each domain as a separate committed snapshot before root policy or workspace conversion:

| Domain | Destination commit | Destination tree | Parent | Imported path | Files |
| --- | --- | --- | --- | --- | ---: |
| Engine | `3fb021c9741ee4cc949992452bc53d0137ff37c1` | `f71e460f03ff9ae1f7213cb53b70377150d081f0` | `ca354d7` | `engine/` | 451 |
| Framework | `fe92c21148bb0df45c93dfa85e8f64e28f44f710` | `3506ee82212342ba8ea47a9704937ce0615f991f` | `3fb021c9741ee4cc949992452bc53d0137ff37c1` | `framework/` | 805 |

The source tree file counts are 451 for engine and 805 for framework. The counts match the destination snapshot paths. The snapshot commits contain no nested `.git` directory or submodule metadata. The root-workspace conversion removes nested product workspace roots; only the app-manifest downstream fixture and the WASM-only `hello_web` example retain isolated workspace metadata by policy.

These destination commits are immutable import facts. They are not final Stage 0 or Stage 1 acceptance evidence. Later root policy, workspace, tooling, CI, and documentation commits must be validated on their final exact source commit.

## Baseline Validation Evidence

The following checks were run before root policy edits:

```text
$ git -C /Users/adityasharma/Projects/rusty/gpui status --porcelain=v1 --untracked-files=all
(empty)
$ git -C /Users/adityasharma/Projects/gpui-component status --porcelain=v1 --untracked-files=all
(empty)
$ git ls-tree -r --name-only 3fb021c9741ee4cc949992452bc53d0137ff37c1 | wc -l
451
$ git ls-tree -r --name-only fe92c21148bb0df45c93dfa85e8f64e28f44f710 | awk '/^framework\//{n++} END{print n}'
805
```

The source repositories remain unchanged after import. No package, release, or tag was created by this migration record.

## Evidence Status

- Stage 0 source foundations exist in the imported snapshots. Final root-workspace, compatibility, package, and release checks remain migration gates.
- Stage 1 source evidence run `30672484082` passed its seven configured jobs before consolidation. That run is historical and does not prove the Neutron source commit.
- Stage 1 must run again on one exact final Neutron commit after root workspace, tooling, CI, and compatibility conversion.
