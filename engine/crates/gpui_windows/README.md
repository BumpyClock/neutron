# gpui_windows

Windows platform backend for the standalone GPUI framework.

This package is part of the Neutron [GPUI engine](https://github.com/BumpyClock/neutron/tree/main/engine) workspace.

## Retained composition

The Direct3D renderer uses offscreen BGRA8 UNORM textures for retained content.
DirectComposition only presents the completed swap chain.
The retained path also operates with `GPUI_DISABLE_DIRECT_COMPOSITION=1`, WARP,
and the rounded host-backdrop presentation mode.

Retained groups preserve scene paint order between ordinary content and sibling
groups. Nested groups apply each child transform before the parent transform.
Each group applies opacity once to its combined child output.
Equal paint ranges follow descriptor order, with the outer descriptor after its
children. The renderer rejects crossed ranges.

Content textures include visible primitive extents, including shadows. The
renderer preserves fractional bounds, rounded content masks, and scene-coordinate
transforms. Identity composition does not repeat inherited rounded coverage.
Transformed composition retains its distinct destination clip. Parent bounds
exclude child source ranges and include only destination-clipped child output.
Empty output needs no texture. Visible blur output retains separate kernel support.
A group uses premultiplied source-over composition in the existing
UNORM target color space. Text inside an isolated group uses grayscale coverage.
Destination-dependent subpixel coverage remains available on the ordinary path.

Cache identity includes the full element ID and its occurrence. Revisions,
geometry, masks, dirty content, and descendant properties invalidate cached
content. For backdrop-independent groups, a change to the group's own opacity or
transform only changes its composition. Removal, resize, device replacement, and a failed layer draw discard
affected textures. Draw failures restore the previous target and viewport.

### Backdrop blur

Element backdrop blur samples preceding scene content. Identity groups at full
opacity retain direct replay for this effect without a new blur-batch boundary.
Transformed or translucent groups render at their paint position, not during
cache preparation. Each group snapshots the current parent pixels, combines any
external ancestor backdrop beneath them, and projects the result into local coordinates.
Each blur snapshot combines this external backdrop with preceding local content
before the existing blur passes execute. Nested transforms, group opacity, and
the blur's own opacity remain separate operations.

Every retained ancestor of a blur bypasses cached pixel reuse. Texture allocations
can be reused, but each frame samples the current parent scene. Blur source bounds
include the existing kernel support padding. Identity wrappers preserve shared
snapshot batches. Unretained element blur and native host-backdrop blur keep their
existing paths. Backdrop-dependent groups incur parent texture copies and projection passes.

Invalid ranges, non-finite descriptor geometry, and texture dimensions above the
device limit also return typed renderer errors. The window reports renderer
errors through its existing error log. A failed frame is not presented.

## Validation

Run CPU geometry, paint-order, cache, and error-contract tests on any supported host:

```sh
cargo test -p gpui_windows --test retained_compositor --locked
```

Run the WARP pixel regression on Windows:

```sh
cargo test --locked -p gpui_windows --lib warp_retained -- --test-threads=1 --nocapture
```

The WARP tests create hidden windows with DirectComposition disabled. They check
cold and warm frames, nested opacity, transforms, ordinary foreground content,
multiple groups, clip changes, overlapping alpha, removal, and resize invalidation
through texture readback. Rounded-mask comparisons cover identity and nested groups.
A fully clipped descendant must not fail the frame or enlarge its parent texture.
Blur regressions compare nested identity groups with
the unretained renderer and check translation, rotation, scale, nested opacity,
local transparent content, and changes to parent colors. These tests do not establish
desktop presentation or hardware-GPU behavior.
The applicable Stage 1 Windows profile remains required for native acceptance.
