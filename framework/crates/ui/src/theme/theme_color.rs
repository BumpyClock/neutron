use std::sync::Arc;

use crate::{ThemeMode, theme::DEFAULT_THEME_COLORS};

use gpui::Hsla;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Theme colors used throughout the UI components.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct ThemeColor {
    /// Used for accents such as hover background on MenuItem, ListItem, etc.
    #[serde(rename = "accent.background")]
    pub accent: Hsla,
    /// Used for accent text color.
    #[serde(rename = "accent.foreground")]
    pub accent_foreground: Hsla,
    /// Accordion background color.
    #[serde(rename = "accordion.background")]
    pub accordion: Hsla,
    /// Accordion hover background color.
    #[serde(rename = "accordion.hover.background")]
    pub accordion_hover: Hsla,
    /// Default background color.
    #[serde(rename = "background")]
    pub background: Hsla,
    /// Default border color
    #[serde(rename = "border")]
    pub border: Hsla,
    /// Background color for GroupBox.
    #[serde(rename = "group_box.background")]
    pub group_box: Hsla,
    /// Text color for GroupBox.
    #[serde(rename = "group_box.foreground")]
    pub group_box_foreground: Hsla,
    /// Input caret color (Blinking cursor).
    #[serde(rename = "caret")]
    pub caret: Hsla,
    /// Chart 1 color.
    #[serde(rename = "chart.1")]
    pub chart_1: Hsla,
    /// Chart 2 color.
    #[serde(rename = "chart.2")]
    pub chart_2: Hsla,
    /// Chart 3 color.
    #[serde(rename = "chart.3")]
    pub chart_3: Hsla,
    /// Chart 4 color.
    #[serde(rename = "chart.4")]
    pub chart_4: Hsla,
    /// Chart 5 color.
    #[serde(rename = "chart.5")]
    pub chart_5: Hsla,
    /// Danger background color.
    #[serde(rename = "danger.background")]
    pub danger: Hsla,
    /// Danger active background color.
    #[serde(rename = "danger.active.background")]
    pub danger_active: Hsla,
    /// Danger text color.
    #[serde(rename = "danger.foreground")]
    pub danger_foreground: Hsla,
    /// Danger hover background color.
    #[serde(rename = "danger.hover.background")]
    pub danger_hover: Hsla,
    /// Description List label background color.
    #[serde(rename = "description_list.label.background")]
    pub description_list_label: Hsla,
    /// Description List label foreground color.
    #[serde(rename = "description_list.label.foreground")]
    pub description_list_label_foreground: Hsla,
    /// Drag border color.
    #[serde(rename = "drag.border")]
    pub drag_border: Hsla,
    /// Drop target background color.
    #[serde(rename = "drop_target.background")]
    pub drop_target: Hsla,
    /// Default text color.
    #[serde(rename = "foreground")]
    pub foreground: Hsla,
    /// Info background color.
    #[serde(rename = "info.background")]
    pub info: Hsla,
    /// Info active background color.
    #[serde(rename = "info.active.background")]
    pub info_active: Hsla,
    /// Info text color.
    #[serde(rename = "info.foreground")]
    pub info_foreground: Hsla,
    /// Info hover background color.
    #[serde(rename = "info.hover.background")]
    pub info_hover: Hsla,
    /// Border color for inputs such as Input, Select, etc.
    #[serde(rename = "input.border")]
    pub input: Hsla,
    /// Link text color.
    #[serde(rename = "link.foreground")]
    pub link: Hsla,
    /// Active link text color.
    #[serde(rename = "link.active.foreground")]
    pub link_active: Hsla,
    /// Hover link text color.
    #[serde(rename = "link.hover.foreground")]
    pub link_hover: Hsla,
    /// Background color for List and ListItem.
    #[serde(rename = "list.background")]
    pub list: Hsla,
    /// Background color for active ListItem.
    #[serde(rename = "list.active.background")]
    pub list_active: Hsla,
    /// Border color for active ListItem.
    #[serde(rename = "list.active.border")]
    pub list_active_border: Hsla,
    /// Stripe background color for even ListItem.
    #[serde(rename = "list.even.background")]
    pub list_even: Hsla,
    /// Background color for List header.
    #[serde(rename = "list.head.background")]
    pub list_head: Hsla,
    /// Hover background color for ListItem.
    #[serde(rename = "list.hover.background")]
    pub list_hover: Hsla,
    /// Muted backgrounds such as Skeleton and Switch.
    #[serde(rename = "muted.background")]
    pub muted: Hsla,
    /// Muted text color, as used in disabled text.
    #[serde(rename = "muted.foreground")]
    pub muted_foreground: Hsla,
    /// Background color for Popover.
    #[serde(rename = "popover.background")]
    pub popover: Hsla,
    /// Text color for Popover.
    #[serde(rename = "popover.foreground")]
    pub popover_foreground: Hsla,
    /// Primary background color.
    #[serde(rename = "primary.background")]
    pub primary: Hsla,
    /// Active primary background color.
    #[serde(rename = "primary.active.background")]
    pub primary_active: Hsla,
    /// Primary text color.
    #[serde(rename = "primary.foreground")]
    pub primary_foreground: Hsla,
    /// Hover primary background color.
    #[serde(rename = "primary.hover.background")]
    pub primary_hover: Hsla,
    /// Progress bar background color.
    #[serde(rename = "progress.bar.background")]
    pub progress_bar: Hsla,
    /// Used for focus ring.
    #[serde(rename = "ring")]
    pub ring: Hsla,
    /// Scrollbar background color.
    #[serde(rename = "scrollbar.background")]
    pub scrollbar: Hsla,
    /// Scrollbar thumb background color.
    #[serde(rename = "scrollbar.thumb.background")]
    pub scrollbar_thumb: Hsla,
    /// Scrollbar thumb hover background color.
    #[serde(rename = "scrollbar.thumb.hover.background")]
    pub scrollbar_thumb_hover: Hsla,
    /// Secondary background color.
    #[serde(rename = "secondary.background")]
    pub secondary: Hsla,
    /// Active secondary background color.
    #[serde(rename = "secondary.active.background")]
    pub secondary_active: Hsla,
    /// Secondary text color, used for secondary Button text color or secondary text.
    #[serde(rename = "secondary.foreground")]
    pub secondary_foreground: Hsla,
    /// Hover secondary background color.
    #[serde(rename = "secondary.hover.background")]
    pub secondary_hover: Hsla,
    /// Input selection background color.
    #[serde(rename = "selection.background")]
    pub selection: Hsla,
    /// Sidebar background color.
    #[serde(rename = "sidebar.background")]
    pub sidebar: Hsla,
    /// Sidebar accent background color.
    #[serde(rename = "sidebar.accent.background")]
    pub sidebar_accent: Hsla,
    /// Sidebar accent text color.
    #[serde(rename = "sidebar.accent.foreground")]
    pub sidebar_accent_foreground: Hsla,
    /// Sidebar border color.
    #[serde(rename = "sidebar.border")]
    pub sidebar_border: Hsla,
    /// Sidebar text color.
    #[serde(rename = "sidebar.foreground")]
    pub sidebar_foreground: Hsla,
    /// Sidebar primary background color.
    #[serde(rename = "sidebar.primary.background")]
    pub sidebar_primary: Hsla,
    /// Sidebar primary text color.
    #[serde(rename = "sidebar.primary.foreground")]
    pub sidebar_primary_foreground: Hsla,
    /// Skeleton background color.
    #[serde(rename = "skeleton.background")]
    pub skeleton: Hsla,
    /// Slider bar background color.
    #[serde(rename = "slider.bar.background")]
    pub slider_bar: Hsla,
    /// Slider thumb background color.
    #[serde(rename = "slider.thumb.background")]
    pub slider_thumb: Hsla,
    /// Success background color.
    #[serde(rename = "success.background")]
    pub success: Hsla,
    /// Success text color.
    #[serde(rename = "success.foreground")]
    pub success_foreground: Hsla,
    /// Success hover background color.
    #[serde(rename = "success.hover.background")]
    pub success_hover: Hsla,
    /// Success active background color.
    #[serde(rename = "success.active.background")]
    pub success_active: Hsla,
    /// Bullish color for candlestick charts (upward price movement).
    #[serde(rename = "bullish.background")]
    pub bullish: Hsla,
    /// Bearish color for candlestick charts (downward price movement).
    #[serde(rename = "bearish.background")]
    pub bearish: Hsla,
    /// Switch background color.
    #[serde(rename = "switch.background")]
    pub switch: Hsla,
    /// Switch thumb background color.
    #[serde(rename = "switch.thumb.background")]
    pub switch_thumb: Hsla,
    /// Tab background color.
    #[serde(rename = "tab.background")]
    pub tab: Hsla,
    /// Tab active background color.
    #[serde(rename = "tab.active.background")]
    pub tab_active: Hsla,
    /// Tab active text color.
    #[serde(rename = "tab.active.foreground")]
    pub tab_active_foreground: Hsla,
    /// TabBar background color.
    #[serde(rename = "tab_bar.background")]
    pub tab_bar: Hsla,
    /// TabBar segmented background color.
    #[serde(rename = "tab_bar.segmented.background")]
    pub tab_bar_segmented: Hsla,
    /// Tab text color.
    #[serde(rename = "tab.foreground")]
    pub tab_foreground: Hsla,
    /// Table background color.
    #[serde(rename = "table.background")]
    pub table: Hsla,
    /// Table active item background color.
    #[serde(rename = "table.active.background")]
    pub table_active: Hsla,
    /// Table active item border color.
    #[serde(rename = "table.active.border")]
    pub table_active_border: Hsla,
    /// Stripe background color for even TableRow.
    #[serde(rename = "table.even.background")]
    pub table_even: Hsla,
    /// Table head background color.
    #[serde(rename = "table.head.background")]
    pub table_head: Hsla,
    /// Table head text color.
    #[serde(rename = "table.head.foreground")]
    pub table_head_foreground: Hsla,
    /// Table item hover background color.
    #[serde(rename = "table.hover.background")]
    pub table_hover: Hsla,
    /// Table row border color.
    #[serde(rename = "table.row.border")]
    pub table_row_border: Hsla,
    /// TitleBar background color, use for Window title bar.
    #[serde(rename = "title_bar.background")]
    pub title_bar: Hsla,
    /// TitleBar border color.
    #[serde(rename = "title_bar.border")]
    pub title_bar_border: Hsla,
    /// Background color for Tiles.
    #[serde(rename = "tiles.background")]
    pub tiles: Hsla,
    /// Warning background color.
    #[serde(rename = "warning.background")]
    pub warning: Hsla,
    /// Warning active background color.
    #[serde(rename = "warning.active.background")]
    pub warning_active: Hsla,
    /// Warning hover background color.
    #[serde(rename = "warning.hover.background")]
    pub warning_hover: Hsla,
    /// Warning foreground color.
    #[serde(rename = "warning.foreground")]
    pub warning_foreground: Hsla,
    /// Overlay background color.
    #[serde(rename = "overlay")]
    pub overlay: Hsla,
    /// Window border color.
    ///
    /// # Platform specific:
    ///
    /// This is only works on Linux, other platforms we can't change the window border color.
    #[serde(rename = "window.border")]
    pub window_border: Hsla,

    /// Fluent TextFillColorDisabled — disabled text color.
    #[serde(rename = "disabled.foreground")]
    pub disabled_foreground: Hsla,
    /// Fluent ControlStrokeColorDefault — subtle control border.
    #[serde(rename = "control.stroke")]
    pub control_stroke: Hsla,
    /// Fluent CardBackgroundFillColorDefault — card surface background.
    #[serde(rename = "card.background")]
    pub card: Hsla,
    /// Card text color.
    #[serde(rename = "card.foreground")]
    pub card_foreground: Hsla,
    /// Fluent SolidBackgroundFillColorBase — solid opaque background.
    #[serde(rename = "solid.background")]
    pub solid_background: Hsla,

    /// The base red color.
    #[serde(rename = "base.red")]
    pub red: Hsla,
    /// The base red light color.
    #[serde(rename = "base.red.light")]
    pub red_light: Hsla,
    /// The base green color.
    #[serde(rename = "base.green")]
    pub green: Hsla,
    /// The base green light color.
    #[serde(rename = "base.green.light")]
    pub green_light: Hsla,
    /// The base blue color.
    #[serde(rename = "base.blue")]
    pub blue: Hsla,
    /// The base blue light color.
    #[serde(rename = "base.blue.light")]
    pub blue_light: Hsla,
    /// The base yellow color.
    #[serde(rename = "base.yellow")]
    pub yellow: Hsla,
    /// The base yellow light color.
    #[serde(rename = "base.yellow.light")]
    pub yellow_light: Hsla,
    /// The base magenta color.
    #[serde(rename = "base.magenta")]
    pub magenta: Hsla,
    /// The base magenta light color.
    #[serde(rename = "base.magenta.light")]
    pub magenta_light: Hsla,
    /// The base cyan color.
    #[serde(rename = "base.cyan")]
    pub cyan: Hsla,
    /// The base cyan light color.
    #[serde(rename = "base.cyan.light")]
    pub cyan_light: Hsla,
}

impl ThemeColor {
    /// Get the default light theme colors.
    pub fn light() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Light].0.clone()
    }

    /// Get the default dark theme colors.
    pub fn dark() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Dark].0.clone()
    }
}
