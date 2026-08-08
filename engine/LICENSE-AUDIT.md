# License and packaging audit

This is an engineering inventory, not a legal opinion. No license was selected or
changed during Phase 0.

## Verified declarations

- GPUI and most extracted engine crates declare `Apache-2.0` in their manifests.
- `zlog`, `ztracing`, and `ztracing_macro` declare `GPL-3.0-or-later`.
- `crates/gpui_shared_string/LICENSE-APACHE` is retained.
- Root `LICENSE-APACHE` and `LICENSE-GPL` texts are restored from the audited
  Zed base and retained through crate license symlinks. There is no root
  `LICENSE-AGPL`; stale AGPL symlinks in tracing crates were removed.

## Publication blockers requiring owner review

1. The engine dependency graph reaches GPL-declared tracing crates through
   `sum_tree`; the owner must decide whether that combined publication is intended
   and which notices must accompany it.
2. `bumpyclock-gpui@0.1.0` is the owner-selected facade identity, but it is not
   published or reserved on crates.io. Its `selected-unpublished` ledger status is
   not a registry ownership claim.
3. `gpui-macros` (0.2.2), `gpui_util` (0.2.2), and `zlog` (0.1.0) already exist in
   the crates.io index under names controlled by other projects. This fork must not
   claim or overwrite those identities; their fork-specific names remain deferred.
4. Bundled fonts and platform assets need a per-release redistribution review; this
   audit does not grant additional rights.

Until these decisions are recorded, release tooling reports registry publication as
blocked even when local package artifacts can be produced.
