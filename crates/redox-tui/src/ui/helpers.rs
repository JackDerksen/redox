//! Small shared UI helpers for editor rendering paths.

use minui::{Color, ColorPair};

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
