//! Line numbers, diagnostic markers and git gutter columns.

use minui::prelude::render::Window;

use crate::GUTTER_CONTENT_PADDING;
use crate::app::EditorState;
use crate::app::state::PaneId;
use crate::ui::UiStyle;
use crate::ui::icons::{
    DIAGNOSTIC_ERROR, DIAGNOSTIC_HINT, DIAGNOSTIC_INFORMATION, DIAGNOSTIC_WARNING,
};
use crate::ui::text_style::TextStyle;

pub(crate) fn draw_active_gutter(
    state: &EditorState,
    background_style: &UiStyle,
    window: &mut dyn Window,
    gutter: &PaneGutterLayout,
    (text_h, first_line, active_cursor_line): (u16, usize, usize),
    highlighted_lines: Option<(usize, usize)>,
) -> minui::Result<()> {
    draw_line_numbers(
        window,
        background_style,
        (gutter.gutter_w, text_h),
        u16::from(gutter.show_marker_column),
        first_line..gutter.total_lines,
        active_cursor_line,
        highlighted_lines,
    )?;
    draw_gutter_padding(
        window,
        background_style,
        gutter.gutter_w,
        text_h,
        GUTTER_CONTENT_PADDING,
        first_line,
        (state.active_git_diff(), Some(state.session.active_buffer())),
    )
}

pub(crate) fn draw_gutter_padding(
    window: &mut dyn Window,
    style: &UiStyle,
    gutter_w: u16,
    text_h: u16,
    padding_w: u16,
    first_line: usize,
    (git_diff, buffer): (
        Option<&crate::app::GitDiffSnapshot>,
        Option<&redox_core::TextBuffer>,
    ),
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let pad = " ".repeat(padding_w as usize);
    let color = TextStyle::new(style.theme.bg, style.theme.bg);
    let staged_style = style.dimmed_by(style.dim_amount).git;
    // Resolve each mark's line once, not once per row.
    let marks = buffer
        .map(|buffer| buffer.marks().collect::<Vec<_>>())
        .unwrap_or_default();
    for row in 0..text_h {
        if padding_w > 0 {
            window.write_str_styled(row, gutter_w, &pad, color.into())?;
        }
        let line_idx = first_line.saturating_add(row as usize);
        if let Some((name, _)) = marks.iter().find(|(_, position)| position.line == line_idx) {
            let mut name_buf = [0; 4];
            window.write_str_styled(
                row,
                0,
                name.encode_utf8(&mut name_buf),
                style.gutter_mark.into(),
            )?;
        } else if let Some(marker) = git_diff.and_then(|diff| diff.marker_for_line(line_idx)) {
            let marker_style = if marker.staged {
                staged_style
            } else {
                style.git
            };
            let (glyph, colors) = marker_style.gutter_marker(marker.kind);
            window.write_str_styled(row, 0, glyph, colors.into())?;
        }
    }
    Ok(())
}

pub(crate) fn diagnostic_icon(severity: crate::app::DiagnosticSeverity) -> &'static str {
    match severity {
        crate::app::DiagnosticSeverity::Error => DIAGNOSTIC_ERROR,
        crate::app::DiagnosticSeverity::Warning => DIAGNOSTIC_WARNING,
        crate::app::DiagnosticSeverity::Information => DIAGNOSTIC_INFORMATION,
        crate::app::DiagnosticSeverity::Hint => DIAGNOSTIC_HINT,
    }
}

pub(crate) fn marker_column_visible(
    buffer: &redox_core::TextBuffer,
    git_diff: Option<&crate::app::GitDiffSnapshot>,
) -> bool {
    buffer.marks().next().is_some() || git_diff.is_some_and(|diff| !diff.stats.is_empty())
}

pub(crate) fn line_number_gutter_width(total_lines: usize, marker_width: u16) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    digits.saturating_add(marker_width).saturating_add(1)
}

pub(crate) fn draw_line_numbers(
    window: &mut dyn Window,
    style: &UiStyle,
    (gutter_w, text_h): (u16, u16),
    marker_width: u16,
    lines: std::ops::Range<usize>,
    cursor_line: usize,
    selected_lines: Option<(usize, usize)>,
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let sep_x = gutter_w.saturating_sub(1);
    let number_w = gutter_w.saturating_sub(marker_width).saturating_sub(1) as usize;
    let other_line_color = style.gutter_line_number;
    let current_color = style.gutter_current_line_number;

    for (row, line_idx) in (0..text_h).zip(lines) {
        let num = if style.layout.line_numbers == crate::ui::style::LineNumbers::Absolute
            || line_idx == cursor_line
        {
            (line_idx + 1).to_string()
        } else {
            line_idx.abs_diff(cursor_line).to_string()
        };

        let clipped_num = if num.chars().count() > number_w {
            num.chars()
                .rev()
                .take(number_w)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        } else {
            num
        };

        let text = format!("{clipped_num:>number_w$}");

        let color = if line_idx == cursor_line
            || selected_lines.is_some_and(|(start, end)| (start..=end).contains(&line_idx))
        {
            current_color
        } else {
            other_line_color
        };

        if number_w > 0 {
            window.write_str_styled(row, marker_width, &text, color.into())?;
        }

        window.write_str_styled(row, sep_x, "▕", color.into())?;
    }

    Ok(())
}

pub(crate) struct PaneGutterLayout {
    pub(crate) content_x: u16,
    pub(crate) gutter_w: u16,
    pub(crate) total_lines: usize,
    pub(crate) show_marker_column: bool,
}

pub(crate) fn pane_gutter_layout(state: &EditorState, pane_id: PaneId) -> PaneGutterLayout {
    let options = state.pane_options(pane_id);
    if !options.has_line_numbers || (state.zen.enabled && state.zen.hide_gutter) {
        return PaneGutterLayout {
            content_x: 0,
            gutter_w: 0,
            total_lines: state.session.active_buffer().len_lines().max(1),
            show_marker_column: false,
        };
    }
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let show_marker_column =
        marker_column_visible(state.session.active_buffer(), state.active_git_diff());
    let gutter_w = line_number_gutter_width(total_lines, u16::from(show_marker_column));
    PaneGutterLayout {
        content_x: gutter_w.saturating_add(GUTTER_CONTENT_PADDING),
        gutter_w,
        total_lines,
        show_marker_column,
    }
}

pub(crate) fn pane_content_x(state: &EditorState, pane_id: PaneId) -> u16 {
    pane_gutter_layout(state, pane_id).content_x
}
