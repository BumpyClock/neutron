//! Shared story-app UI preferences and the real Settings surface.
//!
//! [`StoryUiPreferences`] and [`story_preferences_key`] are exported so a
//! later story example (tsq-11.5.5) can read and write the exact same
//! settings file as the main gallery application. [`story_preferences_module`]
//! is the shared `story.preferences` setup module: it applies the loaded
//! preferences and locale, observes later writes made through
//! [`update_story_preferences`], and refreshes every open window.
//!
//! [`StorySettings`] is the singleton Settings surface: General (locale) and
//! Appearance (theme mode, theme selection, font size, radius, scrollbar
//! behavior, and list active highlighting), built entirely from existing
//! `neutron_components::setting` components and the framework's own
//! `SwitchTheme`/`SwitchThemeMode` actions.
//!
//! This settings file and its backups are never for secrets.

use gpui::{
    App, AppContext as _, Context, Entity, Global, IntoElement, Render, SharedString, Subscription,
    Window, px,
};
use neutron_components::{
    ActiveTheme as _, Theme, ThemeModePreference, ThemeRegistry,
    scroll::ScrollbarShow,
    setting::{SettingControl, SettingGroup, SettingItem, SettingPage, Settings as SettingsPanel},
};
use neutron_components_app::{
    AppSettings, Commands as _, Settings as _, SettingsError, SetupContext, SetupKey, SetupModule,
    StoreKey, SwitchTheme, SwitchThemeMode, shell_preferences, update_shell_preferences,
};
use serde::{Deserialize, Serialize};

/// Persisted UI preferences shared by the story gallery and its examples.
///
/// Types and defaults are preserved from the deleted `title_bar.rs` font/
/// radius/scrollbar/list-highlight controls: font size and radius stay raw
/// pixel counts (not a new enum), matching the exact fixed choices that
/// control offered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoryUiPreferences {
    /// Base font size, in pixels. One of `18`/`16`/`14` (Large/Medium/Small).
    pub font_size: usize,
    /// Corner radius, in pixels. One of `8`/`6`/`4`/`0`.
    pub radius: usize,
    /// When the scrollbar is visible.
    pub scrollbar_show: ScrollbarShow,
    /// Whether the active row in lists is highlighted.
    pub list_active_highlight: bool,
}

impl Default for StoryUiPreferences {
    fn default() -> Self {
        Self {
            font_size: 16,
            radius: 6,
            scrollbar_show: ScrollbarShow::Scrolling,
            list_active_highlight: true,
        }
    }
}

impl AppSettings for StoryUiPreferences {
    const SCHEMA_VERSION: u32 = 1;
}

/// The stable settings-store key shared by every story binary.
pub fn story_preferences_key() -> StoreKey {
    StoreKey::new("story-ui-preferences").expect("valid settings store key")
}

/// Bumped whenever [`update_story_preferences`] persists a change, so the
/// `story.preferences` setup module can reapply it and refresh every open
/// window.
struct StoryPreferencesVersion;

impl Global for StoryPreferencesVersion {}

fn report_settings_error(operation: &str, error: SettingsError) {
    tracing::error!("story settings {operation} failed: {error}");
}

/// Mutate, validate, and persist [`StoryUiPreferences`], then notify the
/// `story.preferences` setup module to reapply it and refresh open windows.
pub fn update_story_preferences(
    cx: &mut App,
    update: impl FnOnce(&mut StoryUiPreferences),
) -> Result<(), SettingsError> {
    cx.update_settings(story_preferences_key(), |preferences, _| {
        update(preferences)
    })?;
    cx.set_global(StoryPreferencesVersion);
    Ok(())
}

/// Persist the platform-owned locale, apply it immediately, and invalidate
/// projected menus so any locale-derived label picks it up.
pub fn update_locale(cx: &mut App, locale: impl Into<String>) -> Result<(), SettingsError> {
    let locale = locale.into();
    update_shell_preferences(cx, |preferences| {
        preferences.locale = Some(locale.clone());
    })?;
    rust_i18n::set_locale(&locale);
    cx.invalidate_menus();
    cx.refresh_windows();
    Ok(())
}

/// Apply the persisted [`StoryUiPreferences`] to the live [`Theme`].
fn apply_story_ui_preferences(cx: &mut App) {
    let preferences = cx
        .settings::<StoryUiPreferences>(story_preferences_key())
        .clone();
    let current = Theme::global(cx);
    if current.font_size == px(preferences.font_size as f32)
        && current.radius == px(preferences.radius as f32)
        && current.scrollbar_show == preferences.scrollbar_show
        && current.list.active_highlight == preferences.list_active_highlight
    {
        return;
    }
    let theme = Theme::global_mut(cx);
    theme.font_size = px(preferences.font_size as f32);
    theme.radius = px(preferences.radius as f32);
    theme.scrollbar_show = preferences.scrollbar_show;
    theme.list.active_highlight = preferences.list_active_highlight;
}

/// Apply the persisted locale, if one was ever chosen.
fn apply_locale(cx: &App) {
    if let Some(locale) = shell_preferences(cx).locale {
        rust_i18n::set_locale(&locale);
    }
}

/// Retained state for the `story.preferences` setup module: the observer
/// subscription that keeps open windows in sync with later preference writes.
pub struct StoryPreferencesState {
    _preferences_subscription: Subscription,
    _theme_subscription: Subscription,
}

fn init_preferences(cx: &mut SetupContext<'_>) -> anyhow::Result<StoryPreferencesState> {
    let app = cx.app();
    if !app.has_global::<StoryPreferencesVersion>() {
        app.set_global(StoryPreferencesVersion);
    }
    // Apply the preferences and locale already loaded by the framework's
    // settings registration, exactly like every later refresh triggered by a
    // write.
    apply_story_ui_preferences(app);
    apply_locale(app);
    app.refresh_windows();
    let preferences_subscription = app.observe_global::<StoryPreferencesVersion>(|cx| {
        apply_story_ui_preferences(cx);
        cx.refresh_windows();
    });
    let theme_subscription = app.observe_global::<Theme>(|cx| {
        apply_story_ui_preferences(cx);
        cx.refresh_windows();
    });
    Ok(StoryPreferencesState {
        _preferences_subscription: preferences_subscription,
        _theme_subscription: theme_subscription,
    })
}

fn teardown_preferences(
    _state: StoryPreferencesState,
    _cx: &mut SetupContext<'_>,
) -> anyhow::Result<()> {
    Ok(())
}

/// The shared `story.preferences` setup module: applies loaded preferences
/// and locale, observes later writes through [`update_story_preferences`],
/// and refreshes every open window. Reusable by every story binary that
/// declares [`StoryUiPreferences`] under [`story_preferences_key`].
pub fn story_preferences_module() -> SetupModule<StoryPreferencesState> {
    SetupModule::new(SetupKey::new("story.preferences"), init_preferences)
        .shutdown(teardown_preferences)
}

/// The story app's real, singleton Settings surface.
///
/// General edits locale; Appearance edits theme mode, theme selection, font
/// size, radius, scrollbar behavior, and list active highlighting. Every
/// control is one of the existing `neutron_components::setting` field types;
/// theme changes dispatch the framework's own `SwitchTheme`/`SwitchThemeMode`
/// actions so persistence and native/in-window menu projection stay owned by
/// the theme convention.
pub struct StorySettings;

impl StorySettings {
    fn setting_pages(&self, _window: &mut Window, cx: &mut Context<Self>) -> Vec<SettingPage> {
        vec![
            SettingPage::new("General").default_open(true).group(
                SettingGroup::new().title("Language").item(
                    SettingItem::new(
                        "Locale",
                        SettingControl::dropdown(
                            vec![
                                ("en".into(), "English".into()),
                                ("zh-CN".into(), "简体中文".into()),
                            ],
                            |_: &App| rust_i18n::locale().to_string().into(),
                            |value: SharedString, cx: &mut App| {
                                if let Err(error) = update_locale(cx, value.to_string()) {
                                    report_settings_error("locale update", error);
                                }
                            },
                        )
                        .default_value("en"),
                    )
                    .description("Choose the display language."),
                ),
            ),
            SettingPage::new("Appearance").groups(vec![
                SettingGroup::new().title("Theme").items(vec![
                    SettingItem::new(
                        "Color Theme",
                        SettingControl::dropdown(
                            theme_set_options(cx),
                            |cx: &App| cx.theme().theme_set_name.clone(),
                            |value: SharedString, cx: &mut App| {
                                cx.dispatch_action(&SwitchTheme(value.to_string()));
                            },
                        )
                        .default_value("Default"),
                    )
                    .description("Select the color theme for the application."),
                    SettingItem::new(
                        "Appearance Mode",
                        SettingControl::dropdown(
                            vec![
                                ("system".into(), "System".into()),
                                ("light".into(), "Light".into()),
                                ("dark".into(), "Dark".into()),
                            ],
                            |cx: &App| mode_to_value(cx.theme().mode_preference),
                            |value: SharedString, cx: &mut App| {
                                cx.dispatch_action(&SwitchThemeMode(mode_from_value(&value)));
                            },
                        )
                        .default_value("system"),
                    )
                    .description("Choose System to follow OS appearance, or force Light/Dark."),
                ]),
                SettingGroup::new().title("Font & Shape").items(vec![
                    SettingItem::new(
                        "Font Size",
                        SettingControl::dropdown(
                            vec![
                                ("18".into(), "Large".into()),
                                ("16".into(), "Medium (default)".into()),
                                ("14".into(), "Small".into()),
                            ],
                            |cx: &App| {
                                cx.settings::<StoryUiPreferences>(story_preferences_key())
                                    .font_size
                                    .to_string()
                                    .into()
                            },
                            |value: SharedString, cx: &mut App| {
                                if let Ok(font_size) = value.parse::<usize>() {
                                    if let Err(error) =
                                        update_story_preferences(cx, |preferences| {
                                            preferences.font_size = font_size;
                                        })
                                    {
                                        report_settings_error("font-size update", error);
                                    }
                                }
                            },
                        )
                        .default_value("16"),
                    )
                    .description("Text size used throughout the gallery."),
                    SettingItem::new(
                        "Border Radius",
                        SettingControl::dropdown(
                            vec![
                                ("8".into(), "8px".into()),
                                ("6".into(), "6px (default)".into()),
                                ("4".into(), "4px".into()),
                                ("0".into(), "0px".into()),
                            ],
                            |cx: &App| {
                                cx.settings::<StoryUiPreferences>(story_preferences_key())
                                    .radius
                                    .to_string()
                                    .into()
                            },
                            |value: SharedString, cx: &mut App| {
                                if let Ok(radius) = value.parse::<usize>() {
                                    if let Err(error) =
                                        update_story_preferences(cx, |preferences| {
                                            preferences.radius = radius;
                                        })
                                    {
                                        report_settings_error("radius update", error);
                                    }
                                }
                            },
                        )
                        .default_value("6"),
                    )
                    .description("Corner rounding used throughout the gallery."),
                ]),
                SettingGroup::new().title("Scrolling & Lists").items(vec![
                    SettingItem::new(
                        "Scrollbar",
                        SettingControl::dropdown(
                            vec![
                                ("scrolling".into(), "Scrolling to show".into()),
                                ("hover".into(), "Hover to show".into()),
                                ("always".into(), "Always show".into()),
                            ],
                            |cx: &App| {
                                scrollbar_to_value(
                                    cx.settings::<StoryUiPreferences>(story_preferences_key())
                                        .scrollbar_show,
                                )
                            },
                            |value: SharedString, cx: &mut App| {
                                let scrollbar_show = scrollbar_from_value(&value);
                                if let Err(error) = update_story_preferences(cx, |preferences| {
                                    preferences.scrollbar_show = scrollbar_show;
                                }) {
                                    report_settings_error("scrollbar update", error);
                                }
                            },
                        )
                        .default_value("scrolling"),
                    )
                    .description("Choose when scrollbars are visible."),
                    SettingItem::new(
                        "List Active Highlight",
                        SettingControl::switch(
                            |cx: &App| {
                                cx.settings::<StoryUiPreferences>(story_preferences_key())
                                    .list_active_highlight
                            },
                            |checked: bool, cx: &mut App| {
                                if let Err(error) = update_story_preferences(cx, |preferences| {
                                    preferences.list_active_highlight = checked;
                                }) {
                                    report_settings_error("list-highlight update", error);
                                }
                            },
                        )
                        .default_value(true),
                    )
                    .description("Highlight the active row in lists."),
                ]),
            ]),
        ]
    }
}

impl Render for StorySettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SettingsPanel::new("story-settings").pages(self.setting_pages(window, cx))
    }
}

fn theme_set_options(cx: &App) -> Vec<(SharedString, SharedString)> {
    ThemeRegistry::global(cx)
        .sorted_theme_sets()
        .iter()
        .map(|set| (set.name.clone(), set.name.clone()))
        .collect()
}

fn mode_to_value(mode: ThemeModePreference) -> SharedString {
    match mode {
        ThemeModePreference::System => "system".into(),
        ThemeModePreference::Light => "light".into(),
        ThemeModePreference::Dark => "dark".into(),
    }
}

fn mode_from_value(value: &str) -> ThemeModePreference {
    match value {
        "light" => ThemeModePreference::Light,
        "dark" => ThemeModePreference::Dark,
        _ => ThemeModePreference::System,
    }
}

fn scrollbar_to_value(show: ScrollbarShow) -> SharedString {
    match show {
        ScrollbarShow::Scrolling => "scrolling".into(),
        ScrollbarShow::Hover => "hover".into(),
        ScrollbarShow::Always => "always".into(),
    }
}

fn scrollbar_from_value(value: &str) -> ScrollbarShow {
    match value {
        "hover" => ScrollbarShow::Hover,
        "always" => ScrollbarShow::Always,
        _ => ScrollbarShow::Scrolling,
    }
}

/// The Settings surface's build function. Unit arguments: the standard
/// Settings command carries no application payload.
pub fn build_settings(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<StorySettings> {
    cx.new(|_| StorySettings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_the_deleted_title_bar_choices() {
        let preferences = StoryUiPreferences::default();
        assert_eq!(
            preferences.font_size, 16,
            "medium was the title bar default"
        );
        assert_eq!(preferences.radius, 6, "6px was the title bar default");
        assert_eq!(preferences.scrollbar_show, ScrollbarShow::Scrolling);
        assert!(preferences.list_active_highlight);
    }

    #[test]
    fn preferences_round_trip_through_json() {
        let preferences = StoryUiPreferences {
            font_size: 18,
            radius: 0,
            scrollbar_show: ScrollbarShow::Always,
            list_active_highlight: false,
        };
        let json = serde_json::to_string(&preferences).expect("serializable");
        let restored: StoryUiPreferences =
            serde_json::from_str(&json).expect("deserializable back");
        assert_eq!(restored.font_size, preferences.font_size);
        assert_eq!(restored.radius, preferences.radius);
        assert_eq!(restored.scrollbar_show, preferences.scrollbar_show);
        assert_eq!(
            restored.list_active_highlight,
            preferences.list_active_highlight
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let restored: StoryUiPreferences =
            serde_json::from_str("{}").expect("every field has a default");
        let defaults = StoryUiPreferences::default();
        assert_eq!(restored.font_size, defaults.font_size);
        assert_eq!(restored.radius, defaults.radius);
        assert_eq!(restored.scrollbar_show, defaults.scrollbar_show);
        assert_eq!(
            restored.list_active_highlight,
            defaults.list_active_highlight
        );
    }
}
