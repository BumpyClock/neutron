use std::path::PathBuf;

use autocorrect::ignorer::Ignorer;
use gpui::{
    App, AppContext, Context, Entity, InteractiveElement, KeyBinding, ParentElement, Render,
    Styled, Window, actions, px,
};

use neutron_components::{
    ActiveTheme as _, IconName, StyledExt as _,
    button::Button,
    dock::PanelControl,
    h_flex,
    label::Label,
    list::ListItem,
    tree::{TreeItem, TreeState, tree},
    v_flex,
};
use rand::seq::SliceRandom as _;

use crate::{Story, section};

actions!(story, [Rename]);

const CONTEXT: &str = "TreeStory";
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("enter", Rename, Some(CONTEXT))]);
}

pub struct TreeStory {
    tree_state: Entity<TreeState>,
    items: Vec<TreeItem>,
}

struct FileRecord {
    id: String,
    label: String,
    children: Vec<FileRecord>,
}

impl FileRecord {
    fn into_tree_item(self) -> TreeItem {
        TreeItem::new(self.id, self.label)
            .children(self.children.into_iter().map(Self::into_tree_item))
    }
}

fn build_file_records(ignorer: &Ignorer, root: &PathBuf, path: &PathBuf) -> Vec<FileRecord> {
    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            let relative_path = path.strip_prefix(root).unwrap_or(&path);
            if ignorer.is_ignored(&relative_path.to_string_lossy())
                || relative_path.ends_with(".git")
            {
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string();
            let id = path.to_string_lossy().to_string();
            let children = if path.is_dir() {
                build_file_records(ignorer, root, &path)
            } else {
                Vec::new()
            };
            items.push(FileRecord {
                id,
                label: file_name,
                children,
            });
        }
    }
    items.sort_by(|a, b| {
        a.children
            .is_empty()
            .cmp(&b.children.is_empty())
            .then(a.label.cmp(&b.label))
    });
    items
}

impl TreeStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn load_files(state: Entity<TreeState>, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |weak_self, cx| {
            let records = cx
                .background_executor()
                .spawn(async move {
                    let ignorer = Ignorer::new(&path.to_string_lossy());
                    build_file_records(&ignorer, &path, &path)
                })
                .await;
            let items: Vec<_> = records
                .into_iter()
                .map(FileRecord::into_tree_item)
                .collect();
            _ = state.update(cx, |state, cx| {
                state.set_items(items.clone(), cx);
            });

            _ = weak_self.update(cx, |this, cx| {
                this.items = items;
                cx.notify();
            })
        })
        .detach();
    }

    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));

        Self::load_files(tree_state.clone(), PathBuf::from("./"), cx);

        Self {
            tree_state,
            items: Vec::new(),
        }
    }

    fn on_action_rename(&mut self, _: &Rename, _: &mut Window, cx: &mut gpui::Context<Self>) {
        if let Some(entry) = self.tree_state.read(cx).selected_entry() {
            let item = entry.item();
            println!("Renaming item: {} ({})", item.label, item.id);
            // Here you could implement actual renaming logic
        }
    }
}

impl Story for TreeStory {
    fn title() -> &'static str {
        "Tree"
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }

    fn zoomable() -> Option<PanelControl> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_records_preserve_hierarchy_and_initial_expansion() {
        let item = FileRecord {
            id: "src".into(),
            label: "src".into(),
            children: vec![
                FileRecord {
                    id: "src/empty".into(),
                    label: "empty".into(),
                    children: Vec::new(),
                },
                FileRecord {
                    id: "src/lib.rs".into(),
                    label: "lib.rs".into(),
                    children: Vec::new(),
                },
            ],
        }
        .into_tree_item();

        assert_eq!(item.id.as_ref(), "src");
        assert!(item.is_folder());
        assert!(!item.is_expanded());
        assert_eq!(item.children[0].id.as_ref(), "src/empty");
        assert!(!item.children[0].is_folder());
        assert!(!item.children[0].is_expanded());
        assert_eq!(item.children[1].label.as_ref(), "lib.rs");
        assert_eq!(item.children[1].id.as_ref(), "src/lib.rs");
    }

    #[gpui::test]
    fn background_file_load_updates_story_and_tree_state(cx: &mut gpui::TestAppContext) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/fixtures");
        let story = cx.new(|cx| {
            let tree_state = cx.new(|cx| TreeState::new(cx));
            TreeStory::load_files(tree_state.clone(), path.clone(), cx);
            TreeStory {
                tree_state,
                items: Vec::new(),
            }
        });
        assert!(story.read_with(cx, |story, _| story.items.is_empty()));
        cx.run_until_parked();
        story.update(cx, |story, cx| {
            let labels: Vec<&str> = story.items.iter().map(|item| item.label.as_ref()).collect();
            assert_eq!(
                labels,
                [
                    "counters.json",
                    "countries.json",
                    "daily-devices.json",
                    "monthly-devices.json",
                    "stock-prices.json",
                ]
            );
            assert!(story.items.iter().all(|item| !item.is_expanded()));
            story.tree_state.update(cx, |state, cx| {
                assert_eq!(state.selected_index(), None);
                state.set_selected_item(Some(&story.items[0]), cx);
                assert_eq!(state.selected_index(), Some(0));
                assert_eq!(state.selected_item().unwrap().id, story.items[0].id);
            });
        });
    }
}

impl Render for TreeStory {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let view = cx.entity();
        v_flex()
            .id("tree-story")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_rename))
            .gap_5()
            .size_full()
            .child(
                h_flex().gap_3().child(
                    Button::new("select-item")
                        .outline()
                        .label("Select Item")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(random_item) = this.items.choose(&mut rand::thread_rng()) {
                                this.tree_state.update(cx, |state, cx| {
                                    state.set_selected_item(Some(random_item), cx);
                                });
                            }
                        })),
                ),
            )
            .child(
                section("File tree")
                    .sub_title("Press `space` to select, `enter` to rename.")
                    .v_flex()
                    .max_w_md()
                    .child(
                        tree(
                            &self.tree_state,
                            move |ix, entry, _selected, _window, cx| {
                                view.update(cx, |_, cx| {
                                    let item = entry.item();
                                    let icon = if !entry.is_folder() {
                                        IconName::File
                                    } else if entry.is_expanded() {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    };

                                    ListItem::new(ix)
                                        .w_full()
                                        .rounded(cx.theme().radius)
                                        .px_3()
                                        .pl(px(16.) * entry.depth() + px(12.))
                                        .child(
                                            h_flex().gap_2().child(icon).child(item.label.clone()),
                                        )
                                        .on_click(cx.listener({
                                            let label = item.label.clone();
                                            let id = item.id.clone();
                                            move |_, _, _window, _| {
                                                println!("Clicked on item: {} ({})", label, id);
                                            }
                                        }))
                                })
                            },
                        )
                        .p_1()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .h(px(540.)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .children(
                                self.tree_state
                                    .read(cx)
                                    .selected_index()
                                    .map(|ix| format!("Selected Index: {}", ix)),
                            )
                            .children(
                                self.tree_state
                                    .read(cx)
                                    .selected_item()
                                    .map(|item| Label::new("Selected:").secondary(item.id.clone())),
                            ),
                    ),
            )
    }
}
