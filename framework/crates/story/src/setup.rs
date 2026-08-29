//! Focused, keyed application setup modules.
//!
//! Each module uses one exact stable key: `story.http-client`, `story.app-state`,
//! and `story.panels`. `story.panels` depends on `story.app-state` so dock
//! restoration never races the registry it restores from. `story.preferences`
//! (applies persisted theme/locale preferences) is declared separately by
//! [`neutron_story::story_preferences_module`], reused across every story
//! binary.

use neutron_components_app::{SetupContext, SetupKey, SetupModule};

const HTTP_CLIENT_KEY: SetupKey = SetupKey::new("story.http-client");
const APP_STATE_KEY: SetupKey = SetupKey::new("story.app-state");
const PANELS_KEY: SetupKey = SetupKey::new("story.panels");

fn init_http_client(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    let http_client =
        std::sync::Arc::new(reqwest_client::ReqwestClient::user_agent("neutron-story")?);
    cx.app().set_http_client(http_client);
    Ok(())
}

/// Installs the story HTTP client.
pub(crate) fn story_http_client_module() -> SetupModule {
    SetupModule::new(HTTP_CLIENT_KEY, init_http_client)
}

fn init_app_state(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    neutron_story::init_app_state(cx.app());
    Ok(())
}

/// Installs the focused `AppState` global and per-story key bindings.
pub(crate) fn story_app_state_module() -> SetupModule {
    SetupModule::new(APP_STATE_KEY, init_app_state)
}

fn init_panels(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    neutron_story::init_panels(cx.app());
    Ok(())
}

/// Registers the `StoryContainer` restore/panel factories. Depends on
/// `story.app-state`: panel restoration reads `AppState` as soon as a
/// persisted dock layout replays.
pub(crate) fn story_panels_module() -> SetupModule {
    SetupModule::new(PANELS_KEY, init_panels).after(APP_STATE_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `SetupModule`'s declared dependencies are `pub(crate)` to the app
    // crate, so `story.panels.after(story.app-state)` itself isn't
    // inspectable from here; `AppDeclaration::validate` is the real check
    // (exercised by the app crate's own declaration tests). The stable key
    // strings below *are* publicly inspectable, so a typo or accidental
    // rename regresses here first.
    #[test]
    fn setup_keys_match_the_documented_exact_strings() {
        assert_eq!(HTTP_CLIENT_KEY.as_str(), "story.http-client");
        assert_eq!(APP_STATE_KEY.as_str(), "story.app-state");
        assert_eq!(PANELS_KEY.as_str(), "story.panels");
    }

    #[test]
    fn every_module_constructs_without_panicking() {
        let _ = story_http_client_module();
        let _ = story_app_state_module();
        let _ = story_panels_module();
    }
}
