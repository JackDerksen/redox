//! Small shared UI helpers for editor rendering paths.

use minui::{Color, ColorPair, TabPolicy, cell_width};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

const TEXT_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

pub(crate) fn animation_progress(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
    }
}

pub(crate) fn smoothstep(progress: f32) -> f32 {
    progress * progress * (3.0 - 2.0 * progress)
}

/// Size a region proportionally, keeping its minimum within the available space.
pub fn proportional_size(available: u16, percent: u16, minimum: u16) -> u16 {
    ((u32::from(available) * u32::from(percent.min(100)) / 100) as u16)
        .max(minimum)
        .min(available)
}

pub fn clip_path_with_filename(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let trimmed = text.trim();
    let path = trimmed
        .split_once(" [loading ")
        .map(|(path, _)| path)
        .unwrap_or(trimmed);
    let filename = path
        .rsplit_once(['/', '\\'])
        .map(|(_, name)| name)
        .unwrap_or(path);
    let filename_len = filename.chars().count();

    if filename_len > max_chars {
        return filename.chars().take(max_chars).collect();
    }

    if max_chars <= filename_len.saturating_add(2) {
        return filename.to_owned();
    }

    if path.chars().count() <= max_chars {
        return path.to_owned();
    }

    let Some((root, separator)) = path_root_component(path) else {
        return filename.to_owned();
    };
    let root_len = root.chars().count();
    let suffix_budget = max_chars.saturating_sub(root_len.saturating_add(3));
    if suffix_budget <= filename_len {
        return filename.to_owned();
    }

    let suffix = path_suffix_components(
        &path[root.len() + separator.len()..],
        separator,
        suffix_budget,
    );
    format!("{root}{separator}…{separator}{suffix}")
}

/// Apply the column background using cell positions relative to the text viewport.
pub fn apply_color_column(
    colors: ColorPair,
    color_column: Option<(usize, Color)>,
    start_cell: usize,
    end_cell: usize,
) -> ColorPair {
    let Some((column, bg)) = color_column else {
        return colors;
    };
    if start_cell <= column && column < end_cell {
        ColorPair::new(colors.fg, bg)
    } else {
        colors
    }
}

fn path_root_component(path: &str) -> Option<(&str, &str)> {
    let mut separators = path.match_indices(['/', '\\']);
    let (first_idx, separator) = separators.next()?;

    if first_idx == 0 {
        let (second_idx, _) = separators.next()?;
        return Some((&path[..second_idx], separator));
    }

    Some((&path[..first_idx], separator))
}

fn path_suffix_components(path_after_root: &str, separator: &str, max_chars: usize) -> String {
    let components: Vec<&str> = path_after_root
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect();
    let Some(filename) = components.last() else {
        return path_after_root.to_owned();
    };

    let mut suffix = (*filename).to_owned();
    let mut suffix_len = suffix.chars().count();
    for component in components[..components.len().saturating_sub(1)]
        .iter()
        .rev()
    {
        let component_len = component.chars().count();
        let candidate_len = suffix_len
            .saturating_add(separator.chars().count())
            .saturating_add(component_len);
        if candidate_len > max_chars {
            break;
        }
        suffix = format!("{component}{separator}{suffix}");
        suffix_len = candidate_len;
    }

    suffix
}

#[inline]
pub(crate) fn cell_width_for_char(ch: char, tab_policy: TabPolicy) -> usize {
    let mut utf8_buffer = [0_u8; 4];
    cell_width(ch.encode_utf8(&mut utf8_buffer), tab_policy) as usize
}

/// Display width of `text`, counting every grapheme as at least one cell.
pub(crate) fn text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| (cell_width(grapheme, TEXT_TAB_POLICY) as usize).max(1))
        .sum()
}

/// The longest suffix of `text` that fits in `max_cells`.
pub(crate) fn clip_text_right_to_cells(text: &str, max_cells: usize) -> String {
    if max_cells == 0 || text.is_empty() {
        return String::new();
    }

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut used = 0usize;
    let mut start = graphemes.len();
    while start > 0 {
        let grapheme_width = text_width(graphemes[start - 1]);
        if used + grapheme_width > max_cells {
            break;
        }
        used += grapheme_width;
        start -= 1;
    }
    graphemes[start..].concat()
}

/// Display width of the first `char_col` characters of `line`.
pub(crate) fn visual_column(line: &str, char_col: usize) -> usize {
    let mut chars_seen = 0usize;
    let mut width = 0usize;
    for grapheme in line.graphemes(true) {
        let next = chars_seen + grapheme.chars().count();
        if char_col < next {
            return width;
        }
        width += cell_width(grapheme, TEXT_TAB_POLICY) as usize;
        chars_seen = next;
        if chars_seen == char_col {
            return width;
        }
    }
    width
}
