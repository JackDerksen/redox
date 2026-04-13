use minui::widgets::WindowView;
use minui::{cell_width, ColorPair, TabPolicy, Window};
use unicode_segmentation::UnicodeSegmentation;

const POPUP_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

#[derive(Debug, Clone, Copy)]
pub struct PopupChrome {
    pub border: ColorPair,
    pub title: ColorPair,
    pub fill: ColorPair,
}

#[derive(Debug, Clone, Copy)]
pub struct PopupLayout {
    pub inner_w: u16,
    pub inner_h: u16,
    pub x: u16,
    pub y: u16,
}

pub fn popup_inner_size(
    term_w: u16,
    term_h: u16,
    width_percent: u16,
    height_percent: u16,
    min_width: u16,
    min_height: u16,
) -> (u16, u16) {
    let popup_w = compute_popup_dim(term_w, width_percent, min_width);
    let popup_h = compute_popup_dim(term_h, height_percent, min_height);
    (popup_w.saturating_sub(2), popup_h.saturating_sub(2))
}

pub fn draw_popup_frame(
    window: &mut dyn Window,
    term_w: u16,
    term_h: u16,
    inner_w: u16,
    inner_h: u16,
    title: &str,
    chrome: PopupChrome,
) -> minui::Result<PopupLayout> {
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    let x = (term_w.saturating_sub(popup_w)) / 2;
    let y = (term_h.saturating_sub(popup_h)) / 2;
    draw_popup_frame_at(window, x, y, inner_w, inner_h, title, chrome)
}

pub fn draw_popup_frame_at(
    window: &mut dyn Window,
    x: u16,
    y: u16,
    inner_w: u16,
    inner_h: u16,
    title: &str,
    chrome: PopupChrome,
) -> minui::Result<PopupLayout> {
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    let horizontal = "─".repeat(popup_w.saturating_sub(2) as usize);
    window.write_str_colored(y, x, &format!("╭{}╮", horizontal), chrome.border)?;
    if popup_h > 1 {
        for row in (y + 1)..(y + popup_h.saturating_sub(1)) {
            window.write_str_colored(row, x, "│", chrome.border)?;
            window.write_str_colored(row, x + popup_w.saturating_sub(1), "│", chrome.border)?;
        }
    }
    if popup_h > 1 {
        window.write_str_colored(
            y + popup_h.saturating_sub(1),
            x,
            &format!("╰{}╯", horizontal),
            chrome.border,
        )?;
    }

    if popup_w > 3 {
        let title_max = popup_w.saturating_sub(4) as usize;
        let title_text = clip_with_ellipsis(title, title_max);
        window.write_str_colored(y, x + 2, &title_text, chrome.title)?;
    }

    if inner_w > 0 && inner_h > 0 {
        let blank_row = " ".repeat(inner_w as usize);
        for row in 0..inner_h {
            window.write_str_colored(y + 1 + row, x + 1, &blank_row, chrome.fill)?;
        }
    }

    Ok(PopupLayout {
        inner_w,
        inner_h,
        x,
        y,
    })
}

pub fn popup_window_view<'a>(window: &'a mut dyn Window, layout: PopupLayout) -> WindowView<'a> {
    WindowView {
        window,
        x_offset: layout.x + 1,
        y_offset: layout.y + 1,
        scroll_x: 0,
        scroll_y: 0,
        width: layout.inner_w,
        height: layout.inner_h,
    }
}

pub fn wrap_text_to_cells(text: &str, max_cells: usize) -> Vec<String> {
    if max_cells == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for raw_line in text.lines() {
        let wrapped = wrap_line_to_cells(raw_line, max_cells);
        if wrapped.is_empty() {
            out.push(String::new());
        } else {
            out.extend(wrapped);
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub fn clip_text_to_cells(text: &str, max_cells: usize) -> String {
    if max_cells == 0 || text.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for g in text.graphemes(true) {
        let gw = (cell_width(g, POPUP_TAB_POLICY) as usize).max(1);
        if used + gw > max_cells {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out
}

fn compute_popup_dim(total: u16, percent: u16, min: u16) -> u16 {
    if total == 0 {
        return 0;
    }

    let desired = ((u32::from(total) * u32::from(percent)) / 100) as u16;
    let floor = min.min(total);
    let ceiling = if total > 2 { total - 2 } else { total };
    desired.max(floor).min(ceiling.max(floor))
}

fn clip_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }

    let mut clipped: String = text.chars().take(max_chars - 3).collect();
    clipped.push_str("...");
    clipped
}

fn wrap_line_to_cells(line: &str, max_cells: usize) -> Vec<String> {
    if line.trim().is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for token in line.split_word_bounds() {
        let is_space = token.chars().all(char::is_whitespace);
        let token_w = text_cell_width(token);

        if is_space {
            if current.is_empty() {
                continue;
            }
            if current_w + token_w <= max_cells {
                current.push_str(token);
                current_w += token_w;
            } else {
                out.push(current.trim_end().to_string());
                current.clear();
                current_w = 0;
            }
            continue;
        }

        if token_w <= max_cells {
            if current_w + token_w <= max_cells {
                current.push_str(token);
                current_w += token_w;
            } else {
                if !current.trim_end().is_empty() {
                    out.push(current.trim_end().to_string());
                }
                current.clear();
                current.push_str(token);
                current_w = token_w;
            }
            continue;
        }

        if !current.trim_end().is_empty() {
            out.push(current.trim_end().to_string());
        }
        current.clear();
        current_w = 0;

        for chunk in hard_wrap_token(token, max_cells) {
            out.push(chunk);
        }
    }

    if !current.trim_end().is_empty() {
        out.push(current.trim_end().to_string());
    }

    out
}

fn hard_wrap_token(token: &str, max_cells: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for g in token.graphemes(true) {
        let gw = (cell_width(g, POPUP_TAB_POLICY) as usize).max(1);
        if current_w + gw > max_cells && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push_str(g);
        current_w += gw;
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn text_cell_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|g| (cell_width(g, POPUP_TAB_POLICY) as usize).max(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_text_wraps_words_across_lines() {
        let wrapped = wrap_text_to_cells("hello world from redox", 11);
        assert_eq!(wrapped, vec!["hello world", "from redox"]);
    }

    #[test]
    fn wrap_text_hard_wraps_long_tokens() {
        let wrapped = wrap_text_to_cells("superlongtoken", 5);
        assert_eq!(wrapped, vec!["super", "longt", "oken"]);
    }

    #[test]
    fn clip_text_respects_cell_width_limit() {
        let clipped = clip_text_to_cells("abcdefgh", 4);
        assert_eq!(clipped, "abcd");
    }
}
