---
title: "Theme"
summary: "How to use GPUI Component theme colors, theme registry, and runtime theme switching."
order: -4
---

# Theme

All components support theming through the built-in Theme system, the [ActiveTheme] trait provides access to the current theme colors:

```rs
use gpui_component::{ActiveTheme as _};

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
use gpui_component::{FlyoutTokens, flyout_primary_foreground, flyout_secondary_foreground};

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

`theme.motion` is four durations, one spring, and four easing curves — every
animated surface draws from this set so the system moves as one:

- `fade_duration_ms` (83) — micro state changes: tooltips, carets, tab strips.
- `exit_duration_ms` (167) — every dismiss. Presence close windows use the same
  token as the close animations, so an exit always plays to completion before
  its element unmounts.
- `enter_duration_ms` (187) — every reveal, and the settle window for the
  spring, so a fade and its transform partner end together.
- `emphasis_duration_ms` (667) — the overshoot accent (badges), used sparingly.
- `spring_damping_ratio` / `spring_frequency` (0.78 / 2.0) — the one spring for
  transform reveals.
- Easings: `decelerate_easing` for enters, `standard_easing` for
  point-to-point moves and exits, `emphasis_easing` for the overshoot,
  `fade_easing` (linear).

Flyouts share one motion grammar: enter = spring slide from the trigger plus a
standard fade over `enter`; exit = standard fade over `exit`. Use
`animation::enter_animation` / `exit_animation` / `spring_animation` /
`standard_animation` / `fade_animation` rather than raw durations, and
`animation::enter_duration` / `exit_duration` for presence windows.

## AppShell integration

AppShell can initialize the registry, persist mode/name in its
`shell-preferences` store, and project theme choices into `StandardMenus`:

```rs
AppShell::builder(APP_IDENTITY)
    .theme(ThemeSource::registry())
    .standard_menus(StandardMenus::new().with_theme_menu())
    // ...
```

Calling `.theme(...)` opts into shell preferences automatically. Apps without a
theme or explicit `.shell_preferences()` consumer do not create that store.

## Material Surfaces

Menus and dropdowns use the flyout material from `SurfacePreset::flyout()`. Its
default backdrop blur extent is 24px, with popover tint opacity of 0.86 in light
mode and 0.88 in dark mode. Themes can override these values through
`material.flyout_blur_radius`, `material.flyout_light_opacity`, and
`material.flyout_dark_opacity`.

## Theme Registry

There have more than 20 built-in themes available in [themes](https://github.com/BumpyClock/gpui-component/tree/main/themes) folder.

https://github.com/BumpyClock/gpui-component/tree/main/themes

And we have a [ThemeRegistry] to help us to load themes.

```rs
use std::path::PathBuf;
use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeRegistry};

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

## Mode Preference

Users can control how theme variants are applied using [ThemeModePreference]:

- **`System`**: Automatically follows the OS appearance setting. When the system switches between light and dark mode, the theme updates accordingly.
- **`Light`**: Always uses the light variant of the selected theme set.
- **`Dark`**: Always uses the dark variant of the selected theme set.

## Fallback Behavior

If a theme set only provides one variant (e.g., only dark), that variant is used for both light and dark modes. This ensures themes always render correctly even if they don't provide both variants.

## Usage Example

```rs
use gpui_component::{Theme, ThemeModePreference, ThemeRegistry};

// Apply a theme set with System mode (auto light/dark switching)
if let Some(set) = ThemeRegistry::global(cx).theme_sets().get("Solarized") {
    Theme::apply_theme_set(set, ThemeModePreference::System, Some(window), cx);
}

// Or apply with a fixed mode
if let Some(set) = ThemeRegistry::global(cx).theme_sets().get("Ayu") {
    Theme::apply_theme_set(set, ThemeModePreference::Dark, Some(window), cx);
}
```

[ActiveTheme]: https://docs.rs/gpui-component/latest/gpui_component/theme/trait.ActiveTheme.html
[ThemeRegistry]: https://docs.rs/gpui-component/latest/gpui_component/theme/struct.ThemeRegistry.html
[ThemeSet]: https://docs.rs/gpui-component/latest/gpui_component/theme/struct.ThemeSet.html
[ThemeModePreference]: https://docs.rs/gpui-component/latest/gpui_component/theme/enum.ThemeModePreference.html
[App]: https://docs.rs/gpui/latest/gpui/struct.App.html
