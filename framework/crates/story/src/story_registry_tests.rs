use super::{StoryState, story_descriptors};
use gpui::AppContext as _;

const EXPECTED_GALLERY_STORIES: &[(&str, &str)] = &[
    ("Getting Started", "WelcomeStory"),
    ("Components", "AccordionStory"),
    ("Components", "AlertDialogStory"),
    ("Components", "AlertStory"),
    ("Components", "AppShellStory"),
    ("Components", "AvatarStory"),
    ("Components", "BadgeStory"),
    ("Components", "BreadcrumbStory"),
    ("Components", "ButtonStory"),
    ("Components", "CalendarStory"),
    ("Components", "ChartStory"),
    ("Components", "CheckboxStory"),
    ("Components", "ClipboardStory"),
    ("Components", "CollapsibleStory"),
    ("Components", "ColorPickerStory"),
    ("Components", "ComboboxStory"),
    ("Components", "CommandPaletteStory"),
    ("Components", "DatePickerStory"),
    ("Components", "DescriptionListStory"),
    ("Components", "DialogStory"),
    ("Components", "DividerStory"),
    ("Components", "DropdownButtonStory"),
    ("Components", "FormStory"),
    ("Components", "GroupBoxStory"),
    ("Components", "HoverCardStory"),
    ("Components", "IconStory"),
    ("Components", "ImageStory"),
    ("Components", "InputStory"),
    ("Components", "KbdStory"),
    ("Components", "LabelStory"),
    ("Components", "ListStory"),
    ("Components", "MenuStory"),
    ("Components", "NotificationStory"),
    ("Components", "NumberInputStory"),
    ("Components", "OtpInputStory"),
    ("Components", "PaginationStory"),
    ("Components", "PopoverStory"),
    ("Components", "ProgressStory"),
    ("Components", "RadioStory"),
    ("Components", "RatingStory"),
    ("Components", "ResizableStory"),
    ("Components", "ScrollbarStory"),
    ("Components", "SelectStory"),
    ("Components", "SettingsStory"),
    ("Components", "SheetStory"),
    ("Components", "FloatingSidebarStory"),
    ("Components", "SidebarStory"),
    ("Components", "SkeletonStory"),
    ("Components", "SliderStory"),
    ("Components", "SpinnerStory"),
    ("Components", "StatusBarStory"),
    ("Components", "StepperStory"),
    ("Components", "SwitchStory"),
    ("Components", "TableStory"),
    ("Components", "TabsStory"),
    ("Components", "TagStory"),
    ("Components", "TextareaStory"),
    ("Components", "ThemeColorsStory"),
    ("Components", "ToggleStory"),
    ("Components", "TooltipStory"),
    ("Components", "TreeStory"),
    ("Components", "VirtualListStory"),
];

#[test]
fn every_gallery_descriptor_can_restore() {
    let descriptors = story_descriptors();
    assert_eq!(descriptors.len(), EXPECTED_GALLERY_STORIES.len());

    for (descriptor, &(expected_group, expected_story_klass)) in
        descriptors.iter().zip(EXPECTED_GALLERY_STORIES)
    {
        assert_eq!(descriptor.group, expected_group);
        assert_eq!(descriptor.story_klass, expected_story_klass);

        let state: StoryState = serde_json::from_value(serde_json::json!({
            "story_klass": expected_story_klass,
        }))
        .unwrap();

        let restored = state
            .descriptor()
            .unwrap_or_else(|| panic!("missing restore descriptor for {expected_story_klass}"));

        assert!(std::ptr::eq(restored, descriptor));
    }
}

#[gpui::test]
fn every_gallery_restore_factory_builds_story(cx: &mut gpui::TestAppContext) {
    let window = cx.update(|cx| {
        super::init(cx);
        cx.open_window(Default::default(), |_, cx| cx.new(|_| gpui::Empty))
            .unwrap()
    });
    let mut visual_cx = gpui::VisualTestContext::from_window(window.into(), cx);

    visual_cx.update(|window, cx| {
        for descriptor in super::story_descriptors() {
            let restored = descriptor.restore(window, cx);
            assert!(!restored.title.is_empty());
            assert_eq!(restored.story_klass, descriptor.story_klass);
        }
    });
}
