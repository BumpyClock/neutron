# Framework upstream relationship

Neutron maintains a selective fork of GPUI Component from
[`longbridge/gpui-component`](https://github.com/longbridge/gpui-component). Neutron preserves
Longbridge attribution and license notices. This document records semantic adaptations. It does
not claim a merge, a byte-identical import, or imported Git history.

## Source identities

The monorepo imported this historical BumpyClock framework snapshot:

- Repository: `https://github.com/BumpyClock/gpui-component`
- Commit: `a3ff1b3132f1b12aee463dd94ece22271b98fd72`
- Tree: `4bc1539532055e69abea2b9740d72b3d262063f8`
- Branch: `main`
- Tag: `v0.6.0`

That snapshot contains Neutron additions. It is historical import provenance, not a Longbridge
audit target.

The 2026-08-23 Longbridge audit used these identities:

- Repository: `https://github.com/longbridge/gpui-component`
- Recorded cursor: `94fdac9b6b762cbe9f23cf91c3fbddb66b80fba3`
- Recorded cursor tree: `46f818bca6e108e55de110081c6c8079582a142c`
- Audited target: `334bbed2e8c47d606eb79ab05ddcebd60b823429`
- Audited target tree: `eef12715845645e98c7d7b2cd276e88d2aba3768`
- Audited target parent: `fe6ef87e5804eb98c613db322052b25fa9b5b56e`
- Target timestamp: `2026-08-24T00:08:54+08:00`

The cursor is an ancestor of the target. The audit inventoried 446 commits. It classified 384
commits as implementation-relevant. Review used the local object database in
`/tmp/longbridge-gpui-component/.git` and compared behavior against Neutron architecture.
The corrected input/text ref `6903579a817cacad3078005f1458e75a4f3291b9` and chart ref
`5af6a197731fbf82a1cad4b8be13f36dcffb6bef` resolve to commit objects and are ancestors of the
audited target.

## Accepted adaptations

The following upstream commits supplied accepted semantics. A commit can appear in more than one
area when it changed shared behavior.

### Input and text

These changes supplied newline normalization, programmatic value behavior, tree-sitter edit
correction, visible-range wrapping, IME ranges, UTF-8 clipping, and selection endpoint fixes.

- `99ac437b80ae941714b9b352226480b7a41030a2`
- `b67d4ef84c615e27e69c2692893477a1b7f0ab6d`
- `e6272f4f1d3e158621868b3cb02be1aa7c5964d4`
- `a9a7341c35b62f27ff512371c62419342264710c`
- `4ac87b15c4df739a2fbca8c4116a920d9e0bb4d6`
- `de5859b271886af45cfab1f8984938d615b0b741`
- `e5b8a3f496e4c812b1548229dc043ac1f72cee1f`
- `6903579a817cacad3078005f1458e75a4f3291b9`
- `35e32845f945e6fd2612228135f6cc83fd37fdf2`
- `cec0defd2f6a0627f494b3b24de075d932e24e76`

### Accessibility and controls

These changes supplied button, input, switch, menu, option, and selection semantics.

- `aaa92dc95ba566e80cfa7426be7e59315301635e`
- `80620f3e730844e02c53bcc17313af9d070765ee`
- `8ee2865dad0e210f69e6649beae895bafcdf9246`
- `f0abdd9f8535c3c7639cc53f666e02318adbf64e`

### Layout and interaction safety

These changes supplied finite resize geometry, dock bounds, list confirmation, and related
interaction guards.

- `06821a18261795e9808d19d98f381886c83a1085`
- `8fbae813c5f265725a729ab6e782c6d3674d8ae9`
- `c7b00f6e879703ca50c0543288cc78127d5bfaff`
- `bfe57805fc35717604f090d33e9040a96c34e50b`

### Focus, dialogs, popovers, and menus

These changes supplied focus restoration, overlay isolation, nested popup priority, action context,
selection cancellation, and recursive menu behavior.

- `3ae8752e845e5f984a932826b664adad227e298b`
- `ca663a2dbe78351728cbe57588cd43e27f87cb48`
- `f19f896fa193c57768eaf5403e4659737e3ba5ba`
- `98a1b6310fd077c84a4c676c58a5657332df96b9`
- `5a2a96e63e22d8d3f3b9131cf6215623a45f2ebf`
- `e12e39ca55051d02f0e2ae8a910a674905c722d6`
- `bc174a7ec4534b2a4174fddde314b38d30d69093`
- `2a90b80800de8bba0e95a4ceb102529eadbc7616`
- `bc3f22927a2c389ec9f53f01a02e3338bb8ff065`
- `c7005a6a2b5df8ee2546d1e96133e6ce0cff456b`

### StatusBar

- `36a6080ad8d098578566c49400217cb1cb8ce3ce`
- `ea6b194db04cc7c0474851f07c7d5b7a9df6a98b`

### AlertDialog

- `e5f5e7ae9580b94f4920161ac00ef39d513b3de6`
- `6ea87a9185e477c4e0b85928fa207f7a4a0670a0`
- `a25c3259112a7897fe74fea0c2be2f34f25ce129`
- `be6575386204ec5560787ff3786c1be7cd344101`
- `5a6896ac8275671b8eb88eb7298316f892dc71c1`
- `890263f82f0ad2a641203d6c6857ae2f08ceeac7`
- `1634c0afc7431ea1f33a7e9bf645f3191af512af`
- `4ad6c7d202e7f662c5c908f34f2abece7d41970d`
- `f0abdd9f8535c3c7639cc53f666e02318adbf64e`

AlertDialog keeps a non-closable overlay. An overlay click does not cancel the prompt. Dialog and
Root retain Neutron focus restoration and deferred-close rules.

### Combobox and SearchableList

- `ce07fbf899ea5ecc25e4bd8599fd3e156b3ed8dc`
- `f2ae9cb3372551768716fa39667b59c9a3b787d0`
- `ccc1e7689203d09bfc88a0aae29473d1c289fa0c`
- `30fc7e768be200c3e20e73a13f601e81a3993971`
- `f0abdd9f8535c3c7639cc53f666e02318adbf64e`
- `5b45bcb26b9343d91a123a4d5ed8a654360512e5`
- `1aab597cae95d631e099f77003f7c3a0c8d1d83a`
- `ab99cfbcb8149910d061896fc10965d4efd3eb94`
- `f07891005fd9db2daace3fe6a1f1e4b57e70e96c`
- `c9be6bf3c8229da274eb588763bd333e5e9573ea`
- `bc3f22927a2c389ec9f53f01a02e3338bb8ff065`
- `1dbd54ff459f6603fa5e4dd4b15eb3e6c837b6f9`
- `b83e4a3a950cfe28a30e4b5a73ad0dd06a821593`
- `c7005a6a2b5df8ee2546d1e96133e6ce0cff456b`
- `81305ef4a0fd86f64777791dd38ead5c303a15f4`
- `2c0931d23caf66c75cd0f271314dd72b9e728772`

Neutron adds value-identity selection, single Confirm emission on Escape, disabled-option guards,
selection remapping after item replacement, accessibility option state, and dynamic-disable
closure. The component keeps Neutron popup priority and focus restoration.

Select remains source-compatible through a thin adapter over SearchableList. Both components use
the same filtering, grouping, cursor state, and default empty renderer. `Select::empty` preserves
its one-use builder contract. `Select::empty_with` rebuilds reusable custom empty content.

## Neutron adaptations

- Keep AppShell, Root, overlay and popup separation, and deferred interaction contracts.
- Keep stable accessibility identifiers and accessibility rollback behavior.
- Keep retained compositor layers, backdrop blur, native blur, material, and motion contracts.
- Map dropdown surfaces to `SurfacePreset::flyout` and existing material tokens.
- Keep components in `framework/crates/ui`. Do not import `gpui-base`.
- Adapt input and text fixes to current engine APIs and tree-sitter integration.
- Preserve local theme schema, gradients, background materials, and platform parity rules.
- Use Neutron popup priority, action context, and focus restoration rules.
- Use native GPUI spring motion only for retargetable Switch geometry.
- Keep fixed-duration presence, dialog, menu, and flyout animation lifetimes.
- Combine engine and framework reduced-motion signals for component animation policy.
- Route duration-based spring easing through the engine `SpringConfig` solver.
- Keep Popover and PopupMenu focus restoration separate because their unfocused-window contracts differ.

## Excluded changes

- `031555662e99a1b5a549990b47f246d475b8288a`: Taffy 0.12 upgrade.
- `b80bb899151cb43db7bdc6f7586736f9fa17253a`: operating-system notifications.
- `1bb129b56297b8a3f5b40f2f52d37838555a2cf9`: `gpui-base` extraction.
- `7b3426db7b357fd7fa77729ebc4dcbcb25656026`: Input, Textarea, and Editor API split.
- `2cadad2274c98172c112de0ec174288bd5725678`: `gpui-base` Dock foundation.
- `8f333a179cd3440f98cc51671322a011ecd4a2fa` and related commits: native menus.
- `010e1a97b5b35e962fabe61b962ebaa3f95d4454`: Table to DataTable rename.
- `cc89092fedc61acc839b4c113eb4778eee12b605` and follow-ups: wholesale Command Palette replacement.
- `e8564ca72ced1a608369a5b843a09f166d4718bb`, `4f99bb8764650eaf6af79163a9dc727675b231ac`, and `5af6a197731fbf82a1cad4b8be13f36dcffb6bef`: lower-priority chart additions.
- Broad manifest, lockfile, CI, website, Nix, rebrand, and dependency changes.
- The macOS `class_addMethod` compatibility shim and silent no-op platform APIs.

## Validation and evidence limits

The 2026-08 validation run passed engine-first unit, workspace, strict Clippy, doctest,
compatibility, headless, framework, story-build, documentation, and macOS native checks. See
`TESTING.md` for evidence levels.

`framework-xtask compatibility check` validates Longbridge identity format. It validates exact
object types, documented tree identities, the direct target parent, and accepted-reference
ancestry. It also validates excluded-reference ancestry. These object checks use
`/tmp/longbridge-gpui-component` without a network fetch. Without that checkout, the check reports
a warning and cannot prove object existence.

The 2026-08 validation run used a checkout without a final destination commit or tree. The run
proves the semantic adaptations that it exercised. It does not prove byte identity with Longbridge.
Exact-source Stage 1 acceptance had not run for that validation, so the run does not provide
exact-source evidence.

Windows native runtime, Linux X11 runtime, Linux Wayland runtime, and browser runtime did not run
locally. Stable WASM stops at `wasm_thread` 0.3.3. Linux musl stops because its C and C++ cross
compilers are absent. All six macOS scenarios passed source-blind validation. The story binary from
that run also presented an on-screen 1324 by 856 AppKit window.

## Next audit

1. Fetch Longbridge objects into a temporary source checkout.
2. Record the current target commit and tree as the next cursor.
3. Inventory implementation commits and changed component paths.
4. Compare behavior against Neutron contracts before each adaptation.
5. Run engine tests before framework and native checks.
6. Update this file only after validation supports the new target.
