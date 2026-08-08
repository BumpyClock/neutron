---
name: apple-design
description: SwiftUI-native guidance for responsive interaction, gesture-driven motion, Liquid Glass, typography, accessibility, and Reduce Motion on iOS, iPadOS, and macOS 26+.
---

# Apple Design

Use this skill for interaction and motion work in Digests. The repo targets iOS, iPadOS, and macOS 26+ and uses SwiftUI first. Keep views declarative, keep expensive work out of `body`, and use the existing design and animation wrappers.

The core test is direct manipulation: feedback starts with the gesture, follows the current presentation value, carries release velocity when appropriate, and remains interruptible.

## 1. Response and direct manipulation

- Give press feedback at touch-down. Commit the action at touch-up unless the control is continuous.
- For drags, update the presented value continuously. Do not wait for the gesture to end.
- Keep transient gesture state in `@GestureState`; keep the committed/resting value in `@State` or an injected observable owner.
- Respect the user's grab offset. Do not snap content to its center when a drag begins.
- Use `DragGesture` and SwiftUI gesture composition. Prefer the smallest `minimumDistance` that avoids accidental activation.
- Resolve competing gestures deliberately with `simultaneousGesture`, `highPriorityGesture`, or `exclusively`, and preserve scrolling when the child gesture has not clearly won.

```swift
struct DraggableCard: View {
    @GestureState private var translation: CGSize = .zero
    @State private var settledOffset: CGSize = .zero

    var body: some View {
        CardContent()
            .offset(
                x: settledOffset.width + translation.width,
                y: settledOffset.height + translation.height
            )
            .gesture(
                DragGesture(minimumDistance: 10)
                    .updating($translation) { value, state, _ in
                        state = value.translation
                    }
                    .onEnded { value in
                        withAnimation(.spring(response: 0.35, dampingFraction: 1.0)) {
                            settledOffset.width += value.translation.width
                            settledOffset.height += value.translation.height
                        }
                    }
            )
    }
}
```

## 2. Interruptible motion

- Animate from the value currently presented on screen. A gesture that starts during a settling animation must take over without a jump.
- Never disable input while an animation is running.
- Use `withAnimation` at the state mutation boundary, not around per-pixel gesture updates or raw scroll deltas.
- Scope `.animation(_:value:)` to a stable, meaningful `Equatable` value. Do not attach an ambient animation to a navigation tree.
- Use critically damped springs by default (`dampingFraction: 1.0`). Use a small amount of bounce only when the user supplied momentum, such as a flick or throw.
- Animate independent axes independently when their targets or velocities differ.
- Keep connected transitions to one to three meaningful elements. Let system navigation transitions do most of the work; do not match text blocks, metadata, or entire complex rows.

For drag release, use SwiftUI's projected endpoint when the interaction should carry momentum, then choose a snap target from that projection. Pass the release direction into the spring decision; do not decide only from the release position.

```swift
.onEnded { value in
    let projectedX = value.predictedEndTranslation.width
    let targetX = nearestSnapPoint(to: settledOffset.width + projectedX)

    withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) {
        settledOffset.width = targetX
    }
}
```

## 3. Sheets, boundaries, and spatial consistency

- Enter and exit along the same path. A sheet that comes from the bottom should dismiss toward the bottom.
- Anchor a popover or sheet to the control that opened it. The source and destination should remain spatially legible.
- Use a small hysteresis threshold before committing a drag direction. Once intent is clear, cancel losing recognizers and track one-to-one.
- At a boundary, rubber-band progressively instead of stopping at a hard edge. Release should settle to a valid bound.
- Keep the UI usable while a transition settles; a new gesture retargets the current state.
- For navigation continuity, follow the repo's connected-transition contracts in `.claude/ai_docs/animation_architecture.md`.

## 4. Timeline and drawing

Use `Canvas` for dense, repeated drawing such as the launch rings. Use `TimelineView` for display-sampled animation. Keep geometry pure and deterministic; do not create one SwiftUI view per glyph or mutate state from a drawing closure.

```pseudocode
TimelineView(animationSchedule) { timeline in
    Canvas { context, size in
        let frame = scene.frame(at: timeline.date, size: size)
        draw(frame, in: &context)
    }
}
```

- Use one timeline and one immutable scene for a field.
- Pause the timeline when the scene reaches a static hold; retain the final frame rather than continuing a no-op clock.
- Keep per-frame work bounded and compositor-friendly. Precompute stable traits and resolve repeated assets once per pass.
- Do not animate layout, large text trees, or unrelated navigation state to make a drawing feel busy.
- Keep `TimelineView`/`Canvas` rendering synchronous and free of I/O, parsing, and task creation.

## 5. Liquid Glass and depth

- App-owned glass goes through `digestsGlassSurface(...)`.
- Group glass-to-glass morphs with `DigestsGlassEffectContainer`; use one namespace per morph group.
- Never scatter raw `glassEffect` calls or availability checks through features. The wrappers provide the iOS Liquid Glass path and the macOS material fallback.
- Do not nest glass surfaces. Use system-owned navigation, tab, and toolbar chrome where the platform provides it.
- Use opaque content surfaces where translucency would reduce reading contrast. Material weight should communicate hierarchy, not decoration.
- Use `glassEffectID` only for a real glass-to-glass morph. Use `matchedGeometryEffect` for non-glass content, and never both for the same element.

```swift
DigestsGlassEffectContainer(spacing: DigestsDesign.Spacing.medium) {
    HStack {
        FilterButton()
        SortButton()
    }
    .digestsGlassSurface(
        useGlass: true,
        cornerRadius: DigestsDesign.CornerRadius.large,
        style: .interactive
    )
}
```

## 6. Accessibility and Reduce Motion

- Read `accessibilityReduceMotion` and route animation decisions through `DigestsDesign.Animation.respectingReduceMotion` or the feature's established policy.
- Reduce Motion keeps causality and state feedback but removes travel, parallax, elastic overshoot, and decorative loops. Prefer a short opacity change or an immediate state change.
- Do not make essential information depend on motion, timing, color, translucency, or haptics.
- Provide an `accessibilityLabel`, `accessibilityValue`, and named `accessibilityAction` for custom controls. Keep VoiceOver focus on the invoking control after a local state change.
- Support Dynamic Type with semantic `Font` styles and layouts that can stack. Test large text and accessibility spacing on iPhone, iPad, and Mac.
- Support keyboard and pointer input on iPad and macOS without making touch gestures the only path.
- Use `sensoryFeedback` only for meaningful commits, errors, or snaps. It must not be the sole confirmation and must not fire for every frame of a gesture.

```swift
struct AnimatedBadge: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isPresented = false

    var body: some View {
        Badge()
            .opacity(isPresented ? 1 : 0)
            .animation(
                reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 1.0),
                value: isPresented
            )
            .accessibilityLabel("New items")
            .accessibilityValue(isPresented ? "Shown" : "Hidden")
    }
}
```

## 7. Typography and layout

- Start with the platform system font and semantic text styles. Use a custom face only for a documented product reason.
- Let Dynamic Type determine text size. Avoid fixed dimensions that truncate or prevent stacking at larger sizes.
- Treat weight, leading, and hierarchy as a set. Use spacing and contrast to group related controls.
- Prefer adaptive SwiftUI layout (`ViewThatFits`, grids, size classes, and platform conditionals) over device-specific coordinates.
- Keep content typography separate from navigation, controls, metadata, and Settings typography. Follow the repo's `readerTypeface` boundary.

## 8. Concurrency and ownership

- Keep presentation state on the main actor. Put network, parsing, persistence, and other heavy work in the existing service or actor owner.
- Use Swift Concurrency with structured tasks, cancellation, and explicit `Sendable` boundaries. Do not start tasks from `body`.
- A timeline or gesture should publish values; it should not become a second data manager, queue, retry loop, or persistence path.
- Preserve the repo's ownership and injection rules. Do not add hidden global fallbacks or parallel animation coordinators.

## Quick reference

| Need | SwiftUI/repo choice |
| --- | --- |
| Immediate press feedback | State mutation at touch-down |
| Continuous drag | `DragGesture` + `@GestureState` |
| Momentum landing | `predictedEndTranslation` + spring |
| Default spring | `.spring(response: 0.3...0.4, dampingFraction: 1.0)` |
| Interrupt a transition | Retarget the current presented state |
| Dense animated drawing | `TimelineView` + one `Canvas` |
| Static hold | Pause the timeline and retain the settled frame |
| App-owned glass | `digestsGlassSurface(...)` |
| Glass morph grouping | `DigestsGlassEffectContainer` |
| Reduced motion | Cross-fade or settle immediately |
| Custom control semantics | Label, value, and named accessibility action |
| Connected navigation | Follow `animation_architecture.md`; match only meaningful elements |

## Process

1. Define the user's intent, the direct manipulation value, and the committed state.
2. Prototype the gesture and its interruption path before polishing materials or decoration.
3. Test touch, keyboard/pointer, VoiceOver, Dynamic Type, Reduce Motion, light/dark appearance, rotation, and split-view changes.
4. Check for ownership violations: no work in `body`, no duplicate manager, no raw glass API, no ambient navigation animation.
5. Review motion at normal speed and in slow motion. Keep only motion that explains state, direction, hierarchy, or completion.
