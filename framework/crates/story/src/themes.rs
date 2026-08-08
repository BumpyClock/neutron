use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{Action, App, SharedString};
use gpui_component::{
    ActiveTheme, Theme, ThemeModePreference, ThemeRegistry, scroll::ScrollbarShow,
};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "target/state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    theme_set: SharedString,
    mode_preference: ThemeModePreference,
    scrollbar_show: Option<ScrollbarShow>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            theme_set: "Default".into(),
            mode_preference: ThemeModePreference::System,
            scrollbar_show: None,
        }
    }
}

fn load_state(path: impl AsRef<Path>) -> Result<Option<State>> {
    let path = path.as_ref();
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read theme state at {}", path.display()));
        }
    };

    let state = serde_json::from_str::<State>(&json)
        .with_context(|| format!("failed to parse theme state at {}", path.display()))?;
    Ok(Some(state))
}

fn persist_state(path: impl AsRef<Path>, state: &State) -> Result<()> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(state).context("failed to serialize theme state")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create theme state directory {}",
                parent.display()
            )
        })?;
    }
    std::fs::write(path, json)
        .with_context(|| format!("failed to write theme state to {}", path.display()))?;
    Ok(())
}

pub fn init(cx: &mut App) {
    // Load last theme state
    tracing::info!("Load themes...");
    let state = match load_state(STATE_FILE) {
        Ok(Some(state)) => state,
        Ok(None) => {
            tracing::info!("No saved theme state found at {STATE_FILE}, using defaults");
            State::default()
        }
        Err(err) => {
            tracing::error!("{err:#}");
            State::default()
        }
    };
    if let Err(err) = ThemeRegistry::watch_dir(PathBuf::from("./themes"), cx, move |cx| {
        if let Some(set) = ThemeRegistry::global(cx)
            .theme_sets()
            .get(&state.theme_set)
            .cloned()
        {
            Theme::apply_theme_set(&set, state.mode_preference, None, cx);
        }
    }) {
        tracing::error!("Failed to watch themes directory: {}", err);
    }

    if let Some(scrollbar_show) = state.scrollbar_show {
        Theme::global_mut(cx).scrollbar_show = scrollbar_show;
    }
    cx.refresh_windows();

    cx.observe_global::<Theme>(|cx| {
        let state = State {
            theme_set: cx.theme().theme_set_name.clone(),
            mode_preference: cx.theme().mode_preference,
            scrollbar_show: Some(cx.theme().scrollbar_show),
        };

        if let Err(err) = persist_state(STATE_FILE, &state) {
            tracing::error!("{err:#}");
        }
    })
    .detach();

    cx.on_action(|switch: &SwitchTheme, cx| {
        let set_name = switch.0.clone();
        if let Some(set) = ThemeRegistry::global(cx)
            .theme_sets()
            .get(&set_name)
            .cloned()
        {
            let preference = Theme::global(cx).mode_preference;
            Theme::apply_theme_set(&set, preference, None, cx);
        }
        cx.refresh_windows();
    });
    cx.on_action(|switch: &SwitchThemeMode, cx| {
        let preference = switch.0;
        let set_name = Theme::global(cx).theme_set_name.clone();
        if let Some(set) = ThemeRegistry::global(cx)
            .theme_sets()
            .get(&set_name)
            .cloned()
        {
            Theme::apply_theme_set(&set, preference, None, cx);
        }
        cx.refresh_windows();
    });
}

#[derive(Action, Clone, PartialEq)]
#[action(namespace = themes, no_json)]
pub(crate) struct SwitchTheme(pub(crate) SharedString);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = themes, no_json)]
pub(crate) struct SwitchThemeMode(pub(crate) ThemeModePreference);

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use gpui_component::{ThemeModePreference, scroll::ScrollbarShow};

    use super::{State, load_state, persist_state};

    fn unique_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("gpui-component-{name}-{nanos}.json"))
    }

    #[test]
    fn load_state_distinguishes_missing_file() {
        let path = unique_path("missing-theme-state");
        let state = load_state(&path).unwrap();
        assert!(state.is_none());
    }

    #[test]
    fn load_state_reports_parse_failure() {
        let path = unique_path("invalid-theme-state");
        std::fs::write(&path, "{ invalid json").unwrap();

        let error = load_state(&path).unwrap_err();
        assert!(error.to_string().contains("failed to parse theme state"));

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn persist_state_creates_parent_directories() {
        let path = unique_path("persist-theme-state");
        let nested = path.parent().unwrap().join("nested").join("state.json");
        let state = State {
            theme_set: "Default".into(),
            mode_preference: ThemeModePreference::Dark,
            scrollbar_show: Some(ScrollbarShow::Hover),
        };

        persist_state(&nested, &state).unwrap();

        let saved = load_state(&nested).unwrap().unwrap();
        assert_eq!(saved.theme_set, state.theme_set);
        assert_eq!(saved.mode_preference, state.mode_preference);
        assert_eq!(saved.scrollbar_show, state.scrollbar_show);

        std::fs::remove_file(&nested).unwrap();
        std::fs::remove_dir(nested.parent().unwrap()).unwrap();
    }
}
