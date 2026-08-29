//! Story-specific commands and menu additions.
//!
//! Declares the app-scoped "Open Repository" command, contributed to Help
//! without duplicating the standard menu, and the window-scoped "Toggle
//! Search" command bound to `/`, scoped to the Gallery's own key context so it
//! never fires outside it.
//!
//! These are declarations only. `story-smoke` evidence never re-resolves them:
//! it reads the *installed* native menu model back through
//! `gpui::App::get_menus` at first presentation, so a declaration that never
//! reaches the platform cannot be recorded as one that did. The tests below
//! stay pure and check the declared functions themselves.

use gpui::{App, actions};
use neutron_components_app::{Command, CommandBinding, CommandId, Menu, MenuBar, MenuKey};

use crate::gallery::GALLERY_KEY_CONTEXT;

actions!(story, [ToggleSearch, OpenRepository]);

const TOGGLE_SEARCH_ID: CommandId = CommandId::new("story.toggle-search");
const OPEN_REPOSITORY_ID: CommandId = CommandId::new("story.open-repository");

fn open_repository(_: &OpenRepository, cx: &mut App) -> anyhow::Result<()> {
    cx.open_url("https://github.com/BumpyClock/neutron");
    Ok(())
}

/// The window-scoped Toggle Search command: `/` while the Gallery's key
/// context is active. No app-level handler; the Gallery's own `.on_action`
/// focuses or defocuses its search input.
pub(crate) fn toggle_search_command() -> Command<ToggleSearch> {
    Command::window(TOGGLE_SEARCH_ID, ToggleSearch)
        .label("Toggle Search")
        .binding(CommandBinding::same("/").key_context(GALLERY_KEY_CONTEXT))
}

/// The app-scoped Open Repository command: no default binding, contributed to
/// Help by [`help_menu_bar`].
pub(crate) fn open_repository_command() -> Command<OpenRepository> {
    Command::app(OPEN_REPOSITORY_ID, OpenRepository, open_repository).label("Open Repository")
}

/// The standard menu bar, with Open Repository contributed to Help alongside
/// the standard content (never a duplicate top-level Help menu).
pub(crate) fn help_menu_bar() -> MenuBar {
    MenuBar::standard().contribute(Menu::keyed(MenuKey::HELP).command(OPEN_REPOSITORY_ID))
}

#[cfg(test)]
mod tests {
    use neutron_components_app::DesktopPlatform;
    use neutron_components_app::commands::standard::{
        CLOSE_WINDOW_COMMAND_ID, COPY_COMMAND_ID, CUT_COMMAND_ID, DELETE_COMMAND_ID,
        DELETE_NEXT_WORD_COMMAND_ID, DELETE_PREVIOUS_WORD_COMMAND_ID, FIND_COMMAND_ID,
        PASTE_COMMAND_ID, QUIT_COMMAND_ID, REDO_COMMAND_ID, SELECT_ALL_COMMAND_ID, UNDO_COMMAND_ID,
    };

    use super::*;

    /// The standard Edit vocabulary every platform's Edit menu must carry.
    const REQUIRED_EDIT_COMMAND_IDS: [CommandId; 10] = [
        UNDO_COMMAND_ID,
        REDO_COMMAND_ID,
        CUT_COMMAND_ID,
        COPY_COMMAND_ID,
        PASTE_COMMAND_ID,
        DELETE_COMMAND_ID,
        DELETE_PREVIOUS_WORD_COMMAND_ID,
        DELETE_NEXT_WORD_COMMAND_ID,
        FIND_COMMAND_ID,
        SELECT_ALL_COMMAND_ID,
    ];

    /// The remaining command ids the resolved outline must project on every
    /// platform: the application block's Quit, the Window menu's Close Window,
    /// and the story's own Help contribution.
    const REQUIRED_STORY_COMMAND_IDS: [CommandId; 3] =
        [QUIT_COMMAND_ID, CLOSE_WINDOW_COMMAND_ID, OPEN_REPOSITORY_ID];

    /// Resolve the declared menu bar for `platform`. Pure: no GPUI, no
    /// platform, no window. Only the tests resolve it; the running binary
    /// observes the installed model instead.
    fn menu_keys(platform: DesktopPlatform) -> Vec<&'static str> {
        outline(platform)
            .menus()
            .iter()
            .map(|menu| menu.key().as_str())
            .collect()
    }

    fn command_ids(platform: DesktopPlatform) -> Vec<&'static str> {
        outline(platform)
            .command_ids()
            .into_iter()
            .map(|command| command.as_str())
            .collect()
    }

    fn outline(platform: DesktopPlatform) -> neutron_components_app::MenuOutline {
        help_menu_bar()
            .outline(platform)
            .expect("the story menu declaration is valid")
    }

    #[test]
    fn every_platform_resolves_its_conventional_menu_order() {
        assert_eq!(
            menu_keys(DesktopPlatform::MacOs),
            vec!["App", "Edit", "Help", "Window"],
            "macOS has no standard Help menu, so the contribution becomes a top-level menu before Window"
        );
        for platform in [DesktopPlatform::Windows, DesktopPlatform::Linux] {
            assert_eq!(
                menu_keys(platform),
                vec!["File", "Edit", "Window", "Help"],
                "{platform:?} contributes into its standard Help menu"
            );
        }
    }

    #[test]
    fn every_platform_projects_the_required_edit_vocabulary() {
        for platform in [
            DesktopPlatform::MacOs,
            DesktopPlatform::Windows,
            DesktopPlatform::Linux,
        ] {
            let command_ids = command_ids(platform);
            for required in REQUIRED_EDIT_COMMAND_IDS
                .iter()
                .chain(REQUIRED_STORY_COMMAND_IDS.iter())
            {
                assert!(
                    command_ids.contains(&required.as_str()),
                    "{platform:?} dropped {required}"
                );
            }
        }
    }

    #[test]
    fn the_story_commands_keep_their_declared_identities() {
        assert_eq!(toggle_search_command().id().as_str(), "story.toggle-search");
        assert_eq!(
            open_repository_command().id().as_str(),
            "story.open-repository"
        );
    }

    #[test]
    fn the_story_actions_keep_their_stable_gpui_names() {
        use gpui::Action as _;

        assert_eq!(ToggleSearch.name(), "story::ToggleSearch");
        assert_eq!(OpenRepository.name(), "story::OpenRepository");
    }
}
