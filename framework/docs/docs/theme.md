---
title: "Theme"
summary: "How to use Neutron Components theme colors, theme registry, and runtime theme switching."
order: -4
---

# Theme

All components support theming through the built-in Theme system, the [ActiveTheme] trait provides access to the current theme colors:

```rs
use neutron_components::{ActiveTheme as _};

// Access theme colors in your components
cx.theme().primary
cx.theme().background
cx.theme().foreground
```

So if you want use the colors from the current theme, you should keep your component or view have [App] context.

## Flyout tokens

Transient surfaces — Popover, HoverCard, PopupMenu, ContextMenu, Select popup,
CommandPalette, the editor popovers and the collapsed sidebar submenu — share one
geometry language via `FlyoutTokens`. `SurfacePreset::flyout()` supplies the
material (blur, noise, elevation, stroke); `FlyoutTokens` supplies the layout on
top of it, so all flyouts stay in the same family when a theme changes.

```rs
use neutron_components::{FlyoutTokens, flyout_primary_foreground, flyout_secondary_foreground};

let tokens = FlyoutTokens::new(cx);          // medium density
let tokens = FlyoutTokens::sized(size, cx);  // match a control's `Size`
```

Key relationships:

- `radius` comes from `theme.radius_lg`; rows use `item_radius = radius - inset`,
  so a row's corner stays concentric with the container's.
- `inset` is the padding between the container edge and its rows; a row's label
  therefore sits at `inset + item_padding_x` from the container edge. Headers,
  footers and search fields inside a flyout should use that same value so every
  surface has a single left rail.
- Two type steps only: `label_size` (Fluent body) for row labels, `meta_size`
  (Fluent caption) for shortcuts, subtitles, categories and section headers.
- The material's tint is theme-derived: the flyout base color is `popover`
  lifted to the material's luminance step (panels lift `sidebar`, the card wash
  lifts `background`), so custom themes tint every surface automatically.
- Text roles come from `flyout_primary_foreground` / `flyout_secondary_foreground`
  (with `flyout_selected_secondary_foreground` on a selected row and
  `flyout_disabled_foreground` for disabled rows) rather than ad-hoc
  `foreground.opacity(..)` values. Both roles are contrast-corrected against the
  composited flyout material — the primary role only in deliberately dim themes,
  the secondary role in most dark themes — and the disabled role sits one clear
  step below the secondary, so labels, supporting text and disabled content stay
  three distinct levels in every theme.

## Motion tokens

`theme.motion` contains four durations, two spring values, and four easing
curves. Components use these values for shared motion timing:

- `fade_duration_ms` (83) — micro state changes: tooltips, carets, tab strips.
- `exit_duration_ms` (167) — every dismiss. Presence close windows use the same
  token as the close animations, so an exit always plays to completion before
  its element unmounts.
- `enter_duration_ms` (187) — every reveal, and the settle window for the
  spring, so a fade and its transform partner end together.
- `emphasis_duration_ms` (667) — the overshoot accent (badges), used sparingly.
- `spring_damping_ratio` / `spring_frequency` (0.78 / 2.0) — normalized spring
  tokens for transform reveals and retargetable control geometry.
- Easings: `decelerate_easing` for enters, `standard_easing` for
  point-to-point moves and exits, `emphasis_easing` for the overshoot,
  `fade_easing` (linear).

Two spring paths serve different lifecycles:

- `animation::spring_animation` samples a spring into GPUI's duration-based
  `Animation`. It is for fixed-duration presence transforms. A target change
  restarts this animation.
- GPUI's stateful `SpringAnimation` with `with_spring` preserves position and
  velocity when a continuously mounted element changes target. `Switch` uses
  this path for its thumb.

Use `animation::enter_animation`, `exit_animation`, `spring_animation`,
`standard_animation`, and `fade_animation` for presence and fixed-duration
motion. Use `enter_duration` and `exit_duration` for presence windows. Keep
backdrop blur, native blur, and material surface behavior separate from motion.

`theme_spring_config` maps normalized theme values to GPUI's physical spring
parameters. For enter duration `d` seconds, frequency `f`, damping ratio `ζ`,
and unit mass, it uses `ω = 2πf/d`, `k = ω²`, `c = 2ζω`, and `m = 1`. Non-finite
or non-positive duration and non-finite spring values use default theme tokens.
Frequency and damping ratio are bounded before conversion.

Reduced motion is enabled when the GPUI engine signal
(`App::reduce_motion()`) is enabled or when a parent `ReducedMotionScope`
provides a reduced-motion value. Components must use
`animation::reduced_motion(cx)` so both signals take effect. Reduced motion
snaps stateful geometry and skips duration-based animations.

`WindowShell` wraps its content in `ReducedMotionScope`. The scope provides its
value before child render, layout, prepaint, and paint, then removes it after
each phase. Use the scope directly when only one subtree must avoid motion:

```rs
use gpui::div;
use neutron_components::{ReducedMotionScope, animation};

ReducedMotionScope::new(true, div().child(my_content));

// A child component can read the combined preference.
let reduce_motion = animation::reduced_motion(cx);
```

`Skeleton` and `Spinner` render without repeat animations when reduced motion is
active. Presence transitions and command-palette reveal delays also stop.

## AppShell integration

`AppDeclaration::new` already installs the registry theme, persists mode and
name in the shell-preferences store, and projects Appearance into the standard
menu bar.

```rs
AppDeclaration::new(APP_IDENTITY)
    .theme(ThemeSource::bundled(app_theme_assets))
    // ...
```

`.theme(...)` replaces the registry source. Shell preferences and Appearance
stay. `.without_theme()` drops the theme source, the shell-preferences store,
and the Appearance section.

## Material Surfaces

Menus and dropdowns use the flyout material from `SurfacePreset::flyout()`. Its
default backdrop blur extent is 24px, with popover tint opacity of 0.86 in light
mode and 0.88 in dark mode. Themes can override these values through
`material.flyout_blur_radius`, `material.flyout_light_opacity`, and
`material.flyout_dark_opacity`.

## Theme Registry

There have more than 20 built-in themes available in [themes](https://github.com/BumpyClock/neutron/tree/main/framework/themes) folder.

https://github.com/BumpyClock/neutron/tree/main/framework/themes

And we have a [ThemeRegistry] to help us to load themes.

```rs
use std::path::PathBuf;
use gpui::{App, SharedString};
use neutron_components::{Theme, ThemeRegistry};

pub fn init(cx: &mut App) {
    let theme_name = SharedString::from("Ayu Light");
    // Load and watch themes from ./themes directory
    if let Err(err) = ThemeRegistry::watch_dir(PathBuf::from("./themes"), cx, move |cx| {
        if let Some(theme) = ThemeRegistry::global(cx)
            .themes()
            .get(&theme_name)
            .cloned()
        {
            Theme::global_mut(cx).apply_config(&theme);
        }
    }) {
        tracing::error!("Failed to watch themes directory: {}", err);
    }
}
```

## Theme Sets

Theme files contain a [ThemeSet] with one or more variants (light and/or dark). The [ThemeRegistry] groups these by theme name, allowing users to select a color theme (e.g., "Solarized") rather than individual variants like "Solarized Light" or "Solarized Dark".

This design allows themes to automatically adapt to the user's appearance preference without requiring separate theme selection for light and dark modes.

## Mode preference

Users can control how theme variants are applied using [ThemeModePreference].
`System` is live. `Theme::apply_theme_set` stores the preference, and each
`Root` watches its window appearance. When the system changes between light and
dark, a `System` root resynchronizes the theme and refreshes all open windows.

Pinned `Light` and `Dark` preferences ignore later appearance notifications.
Use `Root` as the first view in a normal framework window so this subscription
can receive appearance changes.

- **`System`**: Automatically follows the OS appearance setting. When the system switches between light and dark mode, the theme updates accordingly.
- **`Light`**: Always uses the light variant of the selected theme set.
- **`Dark`**: Always uses the dark variant of the selected theme set.

## Fallback Behavior

If a theme set only provides one variant (e.g., only dark), that variant is used for both light and dark modes. This ensures themes always render correctly even if they don't provide both variants.

## Usage Example

```rs
use neutron_components::{Theme, ThemeModePreference, ThemeRegistry};

// Apply a theme set with System mode (auto light/dark switching)
if let Some(set) = ThemeRegistry::global(cx).theme_sets().get("Solarized") {
    Theme::apply_theme_set(set, ThemeModePreference::System, Some(window), cx);
}

// Or apply with a fixed mode
if let Some(set) = ThemeRegistry::global(cx).theme_sets().get("Ayu") {
    Theme::apply_theme_set(set, ThemeModePreference::Dark, Some(window), cx);
}
```

[ActiveTheme]: https://docs.rs/neutron-components/latest/neutron_components/theme/trait.ActiveTheme.html
[ThemeRegistry]: https://docs.rs/neutron-components/latest/neutron_components/theme/struct.ThemeRegistry.html
[ThemeSet]: https://docs.rs/neutron-components/latest/neutron_components/theme/struct.ThemeSet.html
[ThemeModePreference]: https://docs.rs/neutron-components/latest/neutron_components/theme/enum.ThemeModePreference.html
[App]: https://docs.rs/gpui/latest/gpui/struct.App.html
