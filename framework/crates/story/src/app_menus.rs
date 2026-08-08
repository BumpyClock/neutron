use gpui::{App, Entity, Menu, MenuItem, SharedString};
use gpui_component::{
    ActiveTheme as _, Theme, ThemeModePreference, ThemeRegistry, menu::AppMenuBar,
};

use crate::{
    About, Open, Quit, SelectLocale, ToggleSearch,
    themes::{SwitchTheme, SwitchThemeMode},
};

pub fn init(title: impl Into<SharedString>, cx: &mut App) -> Entity<AppMenuBar> {
    let app_menu_bar = AppMenuBar::new(cx);
    let title: SharedString = title.into();
    update_app_menu(title.clone(), app_menu_bar.clone(), cx);

    cx.on_action({
        let title = title.clone();
        let app_menu_bar = app_menu_bar.clone();
        move |s: &SelectLocale, cx: &mut App| {
            rust_i18n::set_locale(&s.0.as_str());
            update_app_menu(title.clone(), app_menu_bar.clone(), cx);
        }
    });

    // Observe theme changes to update the menu to refresh the checked state
    cx.observe_global::<Theme>({
        let app_menu_bar = app_menu_bar.clone();
        move |cx| {
            update_app_menu(title.clone(), app_menu_bar.clone(), cx);
        }
    })
    .detach();

    app_menu_bar
}

fn update_app_menu(title: impl Into<SharedString>, app_menu_bar: Entity<AppMenuBar>, cx: &mut App) {
    let preference = cx.theme().mode_preference;
    cx.set_menus(vec![
        Menu {
            name: title.into(),
            disabled: false,
            items: vec![
                MenuItem::action("About", About),
                MenuItem::Separator,
                MenuItem::action("Open...", Open),
                MenuItem::Separator,
                MenuItem::Submenu(Menu {
                    name: "Appearance".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("System", SwitchThemeMode(ThemeModePreference::System))
                            .checked(preference == ThemeModePreference::System),
                        MenuItem::action("Light", SwitchThemeMode(ThemeModePreference::Light))
                            .checked(preference == ThemeModePreference::Light),
                        MenuItem::action("Dark", SwitchThemeMode(ThemeModePreference::Dark))
                            .checked(preference == ThemeModePreference::Dark),
                    ],
                }),
                theme_menu(cx),
                language_menu(cx),
                MenuItem::Separator,
                MenuItem::action("Quit", Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Undo", gpui_component::input::Undo),
                MenuItem::action("Redo", gpui_component::input::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", gpui_component::input::Cut),
                MenuItem::action("Copy", gpui_component::input::Copy),
                MenuItem::action("Paste", gpui_component::input::Paste),
                MenuItem::separator(),
                MenuItem::action("Delete", gpui_component::input::Delete),
                MenuItem::action(
                    "Delete Previous Word",
                    gpui_component::input::DeleteToPreviousWordStart,
                ),
                MenuItem::action(
                    "Delete Next Word",
                    gpui_component::input::DeleteToNextWordEnd,
                ),
                MenuItem::separator(),
                MenuItem::action("Find", gpui_component::input::Search),
                MenuItem::separator(),
                MenuItem::action("Select All", gpui_component::input::SelectAll),
            ],
        },
        Menu {
            name: "Window".into(),
            disabled: false,
            items: vec![MenuItem::action("Toggle Search", ToggleSearch)],
        },
        Menu {
            name: "Help".into(),
            disabled: false,
            items: vec![MenuItem::action("Open Website", Open)],
        },
    ]);

    app_menu_bar.update(cx, |menu_bar, cx| {
        menu_bar.reload(cx);
    })
}

fn language_menu(_: &App) -> MenuItem {
    let locale = rust_i18n::locale().to_string();
    MenuItem::Submenu(Menu {
        name: "Language".into(),
        disabled: false,
        items: vec![
            MenuItem::action("English", SelectLocale("en".into())).checked(locale == "en"),
            MenuItem::action("简体中文", SelectLocale("zh-CN".into())).checked(locale == "zh-CN"),
        ],
    })
}

fn theme_menu(cx: &App) -> MenuItem {
    let sets = ThemeRegistry::global(cx).sorted_theme_sets();
    let current_set = &cx.theme().theme_set_name;
    MenuItem::Submenu(Menu {
        name: "Theme".into(),
        disabled: false,
        items: sets
            .iter()
            .map(|set| {
                let checked = current_set == &set.name;
                MenuItem::action(set.name.clone(), SwitchTheme(set.name.clone())).checked(checked)
            })
            .collect(),
    })
}
