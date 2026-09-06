# License and packaging audit

This is an engineering inventory, not a legal opinion. Phase 0 did not select or
change a license. The September 2026 upstream review also preserves the current
package declarations and notices.

## Verified declarations

- GPUI and most extracted engine crates declare `Apache-2.0` in their manifests.
- `zlog`, `ztracing`, and `ztracing_macro` declare `GPL-3.0-or-later`.
- `crates/gpui_shared_string/LICENSE-APACHE` is retained.
- Root `LICENSE-APACHE` and `LICENSE-GPL` texts are restored from the audited
  Zed base and retained through crate license symlinks. There is no root
  `LICENSE-AGPL`; stale AGPL symlinks in tracing crates were removed.

## September 2026 upstream grant

Zed commit
[`ac5af8b9e1ea3f7922fbabefe409c05b8766135c`](https://github.com/zed-industries/zed/commit/ac5af8b9e1ea3f7922fbabefe409c05b8766135c)
changes the upstream `zlog`, `ztracing`, and `ztracing_macro` declarations to
`Apache-2.0`. The change also aligns their upstream license files. See
[`zed-industries/zed#63573`](https://github.com/zed-industries/zed/pull/63573).

The Neutron copies are not identical to that upstream source. The retained
`zlog/src/filter.rs` and `zlog/src/sink.rs` differ, including the log rotation
implementation. The retained `ztracing/src/lib.rs` differs in its feature and
WASM code. The `ztracing_macro/src/lib.rs` source matches.

The upstream grant does not, by itself, establish the license of every divergent
historical line in this extraction. A later license migration must reconcile
those differences with an applicable grant or an explicit rights-holder
decision. That migration must preserve attribution and verify the license texts
in package artifacts. This review does not replace the package declarations
with `Apache-2.0` or remove the existing GPL notices.

Track the source reconciliation in
[`BumpyClock/neutron#43`](https://github.com/BumpyClock/neutron/issues/43).

## Publication blockers requiring owner review

1. The engine dependency graph reaches GPL-declared tracing crates through
   `sum_tree`. The owner must resolve the source differences against the new
   upstream grant or decide whether the current combined publication is intended.
   The required package notices remain part of that decision.
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
