use minui::{ColorPair, Window, cell_width};

use crate::input::{WhichKeyEntry, WhichKeyPopup};
use crate::ui::widgets::popup::clip_text_to_cells;
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

// Credit to https://github.com/folke/which-key.nvim for the inspiration here.

const HORIZONTAL_MARGIN: u16 = 1;
const HEADER_ROWS: usize = 1;
const MAX_COLUMNS: usize = 3;
const MIN_COLUMN_WIDTH: usize = 28;
const MAX_BODY_ROWS: usize = 9;
const KEY_WIDTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhichKeyLayout {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl WhichKeyLayout {
    pub fn occludes(self, x: u16, y: u16) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.width)
            && y >= self.y
            && y < self.y.saturating_add(self.height)
    }
}

pub fn draw_which_key_popup(
    popup: &WhichKeyPopup,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<Option<WhichKeyLayout>> {
    let (term_w, term_h) = window.get_size();
    let text_bottom = term_h.saturating_sub(STATUS_BAR_HEIGHT_CELLS);
    let width = term_w.saturating_sub(HORIZONTAL_MARGIN.saturating_mul(2));
    if popup.entries.is_empty() || width < 24 || text_bottom < 3 {
        return Ok(None);
    }

    let columns = ((width.saturating_sub(2) as usize) / MIN_COLUMN_WIDTH)
        .clamp(1, MAX_COLUMNS)
        .min(popup.entries.len());
    let available_body_rows = (text_bottom as usize)
        .saturating_sub(HEADER_ROWS)
        .min(MAX_BODY_ROWS)
        .max(1);
    let capacity = columns.saturating_mul(available_body_rows);
    let entries = visible_entries(&popup.entries, capacity);
    let body_rows = entries.len().div_ceil(columns).max(1);
    let height = (HEADER_ROWS + body_rows) as u16;
    let x = HORIZONTAL_MARGIN;
    let y = text_bottom.saturating_sub(height);
    let layout = WhichKeyLayout {
        x,
        y,
        width,
        height,
    };

    let popup_bg = style.which_key.background;
    let fill = ColorPair::new(style.which_key.text, popup_bg);
    let blank = " ".repeat(width as usize);
    for row in 0..height {
        window.write_str_colored(y + row, x, &blank, fill)?;
        window.write_str_colored(
            y + row,
            x,
            "▌",
            ColorPair::new(style.which_key.edge, popup_bg),
        )?;
    }

    let header = format!(" {} …", popup.prefix);
    let header = clip_text_to_cells(&header, width.saturating_sub(2) as usize);
    window.write_str_colored(
        y,
        x.saturating_add(1),
        &header,
        ColorPair::new(style.which_key.prefix, popup_bg),
    )?;

    let content_width = width.saturating_sub(2) as usize;
    let column_width = (content_width / columns).max(1);
    let key_widths = column_key_widths(&entries, body_rows, column_width);
    for (index, entry) in entries.iter().enumerate() {
        let column = index / body_rows;
        let row = index % body_rows;
        let entry_x = x
            .saturating_add(2)
            .saturating_add((column.saturating_mul(column_width)) as u16);
        let entry_y = y.saturating_add(HEADER_ROWS as u16 + row as u16);
        draw_entry(
            window,
            entry,
            style,
            popup_bg,
            entry_x,
            entry_y,
            column_width,
            key_widths[column],
        )?;
    }

    Ok(Some(layout))
}

fn column_key_widths(
    entries: &[WhichKeyEntry],
    body_rows: usize,
    column_width: usize,
) -> Vec<usize> {
    let max_key_width = KEY_WIDTH.min(column_width.saturating_sub(3));
    entries
        .chunks(body_rows.max(1))
        .map(|column| {
            column
                .iter()
                .map(|entry| cell_width(&entry.key, minui::TabPolicy::Fixed(4)) as usize)
                .max()
                .unwrap_or_default()
                .min(max_key_width)
        })
        .collect()
}

fn visible_entries(entries: &[WhichKeyEntry], capacity: usize) -> Vec<WhichKeyEntry> {
    if entries.len() <= capacity {
        return entries.to_vec();
    }
    if capacity == 0 {
        return Vec::new();
    }

    let mut visible = entries[..capacity].to_vec();
    let hidden = entries.len().saturating_sub(capacity).saturating_add(1);
    visible[capacity - 1] = WhichKeyEntry {
        key: "…".to_string(),
        description: format!("{hidden} more"),
    };
    visible
}

fn draw_entry(
    window: &mut dyn Window,
    entry: &WhichKeyEntry,
    style: UiStyle,
    popup_bg: minui::Color,
    x: u16,
    y: u16,
    width: usize,
    key_width: usize,
) -> minui::Result<()> {
    if width < 4 {
        return Ok(());
    }
    let key = clip_text_to_cells(&entry.key, key_width);
    window.write_str_colored(y, x, &key, ColorPair::new(style.which_key.key, popup_bg))?;

    let key_cells = cell_width(&key, minui::TabPolicy::Fixed(4));
    let key_padding = key_width.saturating_sub(key_cells as usize);
    if key_padding > 0 {
        window.write_str_colored(
            y,
            x.saturating_add(key_cells as u16),
            &" ".repeat(key_padding),
            ColorPair::new(style.which_key.key, popup_bg),
        )?;
    }
    let arrow_x = x.saturating_add(key_width as u16);
    window.write_str_colored(
        y,
        arrow_x,
        " → ",
        ColorPair::new(style.which_key.arrow, popup_bg),
    )?;

    let description_width = width.saturating_sub(key_width).saturating_sub(3);
    let description = clip_text_to_cells(&entry.description, description_width);
    window.write_str_colored(
        y,
        arrow_x.saturating_add(3),
        &description,
        ColorPair::new(style.which_key.text, popup_bg),
    )
}
