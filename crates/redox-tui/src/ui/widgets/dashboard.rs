use std::time::Duration;

use minui::{Style, Window, window::CursorSpec};

use crate::app::state::dashboard::DASHBOARD_ITEMS;
use crate::ui::UiStyle;
use crate::ui::icons::dashboard_icon;
use crate::ui::style::{TextStyle, dim_foreground_color};
use crate::ui::widgets::popup::clip_text_to_cells;

const MIN_VERTICAL_MARGIN_ROWS: u16 = 2;
const LOGO_VERSION_GAP_ROWS: u16 = 1;
const VERSION_MENU_GAP_ROWS: u16 = 1;
const MENU_ITEM_GAP_ROWS: u16 = 1;
const MENU_WIDTH_COLS: u16 = 30;
const ICON_WIDTH_COLS: u16 = 1;
const ICON_LABEL_GAP_COLS: u16 = 2;
const MIN_LABEL_HOTKEY_GAP_COLS: u16 = 1;
const HOTKEY_WIDTH_COLS: u16 = 1;
pub(crate) const LOGO_ENTRANCE_DURATION: Duration = Duration::from_millis(500);

/*
const LOGO_TOP_CONNECTOR: &[&str] = &[
    "████████████████████████████┐",
    "██┌───────────────────────██│",
    "└─┘                       └─┘",
];
const LOGO_WORDMARK: &[&str] = &[
    "███████┐                  ██┐",
    "██┌───██┐                 ██│",
    "██│   ██│  ██████┐   ███████│  ██████┐  ██┐   ██┐",
    "███████┌┘ ██┌───██┐ ██┌───██│ ██┌───██┐  ██┐ ██┌┘",
    "██┌─██┌┘  ████████│ ██│   ██│ ██│   ██│   ████┌┘",
    "██│ └██┐  ██┌─────┘ ██│   ██│ ██│   ██│  ██┌─██┐",
    "██│  └██┐  ██████┐   ███████│  ██████┌┘ ██┌┘  ██┐",
    "└─┘   └─┘  └─────┘   └──────┘  └─────┘  └─┘   └─┘",
];
const LOGO_BOTTOM_CONNECTOR: &[&str] = &[
    "                          ██┐                 ██┐",
    "                          ██████████████████████│",
    "                          └─────────────────────┘",
];
*/
const LOGO_TOP_CONNECTOR: &[&str] = &[
    "████████████████████████████┓",
    "██┏━━━━━━━━━━━━━━━━━━━━━━━██┃",
    "┗━┛                       ┗━┛",
];
const LOGO_WORDMARK: &[&str] = &[
    "███████┓                  ██┓",
    "██┏━━━██┓                 ██┃",
    "██┃   ██┃  ██████┓   ███████┃  ██████┓  ██┓   ██┓",
    "███████┏┛ ██┏━━━██┓ ██┏━━━██┃ ██┏━━━██┓  ██┓ ██┏┛",
    "██┏━██┏┛  ████████┃ ██┃   ██┃ ██┃   ██┃   ████┏┛",
    "██┃ ┗██┓  ██┏━━━━━┛ ██┃   ██┃ ██┃   ██┃  ██┏━██┓",
    "██┃  ┗██┓  ██████┓   ███████┃  ██████┏┛ ██┏┛  ██┓",
    "┗━┛   ┗━┛  ┗━━━━━┛   ┗━━━━━━┛  ┗━━━━━┛  ┗━┛   ┗━┛",
];
const LOGO_BOTTOM_CONNECTOR: &[&str] = &[
    "                          ██┓                 ██┓",
    "                          ██████████████████████┃",
    "                          ┗━━━━━━━━━━━━━━━━━━━━━┛",
];

pub(crate) fn draw_dashboard(
    window: &mut dyn Window,
    style: UiStyle,
    selected: usize,
    show_cursor: bool,
    logo_elapsed: Option<Duration>,
) -> minui::Result<()> {
    let (width, height) = window.get_size();
    if width == 0 || height == 0 {
        return Ok(());
    }

    let logo_sections = [
        (LOGO_TOP_CONNECTOR, style.dashboard.logo_red),
        (LOGO_WORDMARK, style.dashboard.logo_white),
        (LOGO_BOTTOM_CONNECTOR, style.dashboard.logo_blue),
    ];
    let compact_logo = [
        // So cute
        ("┏━┓", style.about.logo_red),
        ("Redox", style.about.logo_white),
        ("  ┗━┛", style.about.logo_blue),
    ];
    let logo_width = logo_sections
        .iter()
        .flat_map(|(lines, _)| lines.iter())
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let full_logo_height = logo_sections
        .iter()
        .map(|(lines, _)| lines.len())
        .sum::<usize>() as u16;
    let compact_logo_height = compact_logo.len() as u16;
    let item_count = DASHBOARD_ITEMS.len() as u16;
    let version_height = LOGO_VERSION_GAP_ROWS + 1 + VERSION_MENU_GAP_ROWS;
    let available_height = height.saturating_sub(MIN_VERTICAL_MARGIN_ROWS * 2);
    let large_logo =
        width >= logo_width && available_height >= full_logo_height + version_height + item_count;
    let logo_height = if large_logo {
        full_logo_height
    } else if available_height >= compact_logo_height + version_height + item_count {
        compact_logo_height
    } else {
        0
    };
    let header_height = if logo_height > 0 {
        logo_height + version_height
    } else {
        0
    };
    let expanded_menu_height = item_count + item_count.saturating_sub(1) * MENU_ITEM_GAP_ROWS;
    let item_gap = if available_height >= header_height + expanded_menu_height {
        MENU_ITEM_GAP_ROWS
    } else {
        0
    };
    let row_step = 1 + item_gap;
    let available_rows = height.saturating_sub(header_height);
    let visible_items = available_rows.div_ceil(row_step).min(item_count);
    let menu_height = visible_items + visible_items.saturating_sub(1) * item_gap;
    let content_height = header_height + menu_height;
    let top = height.saturating_sub(content_height) / 2;
    let icon_prefix_width = if style.icons_enabled {
        ICON_WIDTH_COLS + ICON_LABEL_GAP_COLS
    } else {
        0
    };
    let menu_width = (MENU_WIDTH_COLS + icon_prefix_width).min(width);
    let menu_left = width.saturating_sub(menu_width) / 2;
    let hotkey_column = menu_left + menu_width - HOTKEY_WIDTH_COLS;
    let label_column = menu_left + icon_prefix_width;
    let label_width = hotkey_column.saturating_sub(label_column + MIN_LABEL_HOTKEY_GAP_COLS);
    let progress = logo_elapsed.map_or(1.0, |elapsed| {
        (elapsed.as_secs_f32() / LOGO_ENTRANCE_DURATION.as_secs_f32()).clamp(0.0, 1.0)
    });

    if large_logo {
        let left = (width - logo_width) / 2;
        let lines = logo_sections
            .iter()
            .enumerate()
            .flat_map(|(section, (lines, color))| {
                lines.iter().map(move |text| (section, text, *color))
            });
        for (row, (section, text, color)) in lines.enumerate() {
            draw_logo_line(
                window,
                top + row as u16,
                left,
                text,
                color,
                section,
                progress,
            )?;
        }
    } else if logo_height > 0 {
        let compact_logo_width = compact_logo
            .iter()
            .map(|(text, _)| text.chars().count())
            .max()
            .unwrap_or(0) as u16;
        let left = width.saturating_sub(compact_logo_width) / 2;
        for (row, (text, color)) in compact_logo.iter().enumerate() {
            draw_logo_line(window, top + row as u16, left, text, *color, row, progress)?;
        }
    }
    if logo_height > 0 {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        write_clipped(
            window,
            top + logo_height + LOGO_VERSION_GAP_ROWS,
            width.saturating_sub(version.len() as u16) / 2,
            &version,
            style.dashboard.version,
        )?;
    }

    let first = selected.saturating_sub(visible_items.saturating_sub(1) as usize);
    for (row, (hotkey, label)) in DASHBOARD_ITEMS
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_items as usize)
    {
        let y = top + header_height + (row - first) as u16 * row_step;
        let text_color = if row == selected {
            style.dashboard.selected
        } else {
            style.dashboard.text
        };
        if style.icons_enabled && menu_width > icon_prefix_width {
            write_clipped(
                window,
                y,
                menu_left,
                dashboard_icon(*hotkey),
                style.dashboard.icon,
            )?;
        }
        write_clipped(
            window,
            y,
            label_column,
            &clip_text_to_cells(label, label_width as usize),
            text_color,
        )?;
        write_clipped(
            window,
            y,
            hotkey_column,
            &hotkey.to_string(),
            style.dashboard.hotkey,
        )?;
        if show_cursor && row == selected {
            window.request_cursor(CursorSpec {
                x: hotkey_column,
                y,
                visible: true,
            });
        }
    }
    Ok(())
}

fn draw_logo_line(
    window: &mut dyn Window,
    row: u16,
    column: u16,
    text: &str,
    style: TextStyle,
    section: usize,
    progress: f32,
) -> minui::Result<()> {
    if progress >= 1.0 {
        return write_clipped(window, row, column, text, style);
    }
    let progress = progress * progress * (3.0 - 2.0 * progress);
    let start = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    let end = text.chars().count().saturating_sub(1);
    for (offset, character) in text.chars().enumerate().skip(start) {
        // The bars reveal in opposite directions; the wordmark fades in just behind them.
        let opacity = if section == 1 {
            ((progress - 0.15) / 0.85).clamp(0.0, 1.0)
        } else {
            let distance = if section == 0 {
                offset - start
            } else {
                end - offset
            };
            let distance = distance as f32 / end.saturating_sub(start).max(1) as f32;
            ((progress * 1.3 - distance) / 0.3).clamp(0.0, 1.0)
        };
        if opacity == 0.0 {
            continue;
        }
        let foreground = dim_foreground_color(style.fg, style.bg, 1.0 - opacity);
        let mut encoded = [0; 4];
        write_clipped(
            window,
            row,
            column + offset as u16,
            character.encode_utf8(&mut encoded),
            style.with_colors(foreground, style.bg),
        )?;
    }
    Ok(())
}

fn write_clipped(
    window: &mut dyn Window,
    row: u16,
    column: u16,
    text: &str,
    style: impl Into<Style>,
) -> minui::Result<()> {
    let (width, height) = window.get_size();
    if row < height && column < width {
        window.write_str_styled(
            row,
            column,
            &clip_text_to_cells(text, (width - column) as usize),
            style.into(),
        )?;
    }
    Ok(())
}
