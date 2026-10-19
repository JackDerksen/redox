//! Drawing a single buffer line with highlights, snippets and inline diagnostics.

use std::borrow::Cow;

use minui::prelude::render::{Color, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::UiStyle;
use crate::ui::helpers::apply_color_column;
use crate::ui::overlays::LineDecorations;
use crate::ui::render::LineViewport;
use crate::ui::syntax::syntax_style_for_range;
use crate::ui::text_style::TextStyle;
use crate::view::cells::visible_content_cell_width;
use crate::view::gutter::diagnostic_icon;

pub(crate) struct LineHighlights<'a> {
    pub(crate) color_column: Option<(usize, Color)>,
    pub(crate) layers: &'a [(&'a [bool], Color, Option<Color>)],
    pub(crate) empty_line: bool,
    pub(crate) decorations: LineDecorations<'a>,
}

pub(crate) fn draw_line_with_highlights(
    window: &mut dyn Window,
    viewport: LineViewport,
    source_line: &str,
    normal_color: TextStyle,
    highlights: LineHighlights<'_>,
    style: &UiStyle,
    syntax_spans: Option<&[crate::ui::syntax::LineSyntaxSpan]>,
) -> minui::Result<()> {
    let LineViewport {
        row,
        column: col,
        scroll_x,
        width: width_cells,
    } = viewport;
    let LineHighlights {
        color_column,
        layers: highlight_layers,
        empty_line: highlight_empty_line,
        decorations,
    } = highlights;

    if width_cells == 0 {
        return Ok(());
    }

    if source_line.is_empty() {
        if highlight_empty_line {
            draw_highlight_spaces(
                window,
                viewport,
                0,
                normal_color,
                highlight_layers,
                decorations,
                color_column,
            )?;
            if let Some((visible_col, bg)) = color_column
                && visible_col < width_cells
                && highlight_color_at_cell(highlight_layers, visible_col, normal_color.fg).is_none()
                && decorations
                    .undo_redo_highlight
                    .is_none_or(|(cells, _)| cells.get(visible_col) != Some(&true))
            {
                window.write_str_styled(
                    row,
                    col.saturating_add(visible_col as u16),
                    " ",
                    (normal_color.with_colors(normal_color.fg, bg)).into(),
                )?;
            }
        } else if let Some((visible_col, bg)) = color_column
            && visible_col < width_cells
        {
            window.write_str_styled(
                row,
                col.saturating_add(visible_col as u16),
                " ",
                (normal_color.with_colors(normal_color.fg, bg)).into(),
            )?;
        }
        return Ok(());
    }

    let mut used_cells = 0usize;
    let mut line_cells = 0usize;
    let mut byte_idx = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

    for g in source_line.graphemes(true) {
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let g_bytes = g.len();
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_byte = byte_idx;
        let end_byte = byte_idx.saturating_add(g_bytes);

        line_cells = end_cell;
        byte_idx = end_byte;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell < scroll_x {
            continue;
        }
        if end_cell > max_visible_cell {
            break;
        }

        let visible_start = start_cell.max(scroll_x).saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        let base_style = syntax_spans
            .map(|spans| syntax_style_for_range(normal_color, style, spans, start_byte, end_byte))
            .unwrap_or_else(|| normal_color.into());
        let base_color = base_style.colors.unwrap_or(normal_color.colors());
        let highlight_color =
            highlight_layers
                .iter()
                .find_map(|(cells, background, foreground)| {
                    (visible_start < visible_end
                        && visible_end <= cells.len()
                        && cells[visible_start..visible_end]
                            .iter()
                            .any(|selected| *selected))
                    .then_some(
                        TextStyle::new(foreground.unwrap_or(base_color.fg), *background).colors(),
                    )
                });
        let color = if let Some(color) = highlight_color {
            color
        } else {
            apply_color_column(base_color, color_column, visible_start, visible_end)
        };

        let text_style =
            decorations.apply(base_style.with_colors(color), visible_start..visible_end);
        if g == "\t" {
            for cell in visible_start..visible_end {
                let colors = highlight_color_at_cell(highlight_layers, cell, base_color.fg)
                    .map(|colors| colors.colors())
                    .unwrap_or_else(|| {
                        apply_color_column(base_color, color_column, cell, cell + 1)
                    });
                window.write_str_styled(
                    row,
                    col.saturating_add(cell as u16),
                    " ",
                    decorations.apply(base_style.with_colors(colors), cell..cell + 1),
                )?;
            }
        } else {
            window.write_str_styled(
                row,
                col.saturating_add(visible_start as u16),
                g,
                text_style,
            )?;
        }
        used_cells = visible_end;
    }

    draw_highlight_spaces(
        window,
        viewport,
        used_cells,
        normal_color,
        highlight_layers,
        decorations,
        color_column,
    )?;

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
        && highlight_color_at_cell(highlight_layers, visible_col, normal_color.fg).is_none()
        && decorations
            .undo_redo_highlight
            .is_none_or(|(cells, _)| cells.get(visible_col) != Some(&true))
    {
        window.write_str_styled(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            (normal_color.with_colors(normal_color.fg, bg)).into(),
        )?;
    }

    Ok(())
}

pub(crate) fn draw_highlight_spaces(
    window: &mut dyn Window,
    viewport: LineViewport,
    start_cell: usize,
    normal_color: TextStyle,
    highlight_layers: &[(&[bool], Color, Option<Color>)],
    decorations: LineDecorations<'_>,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
    let LineViewport {
        row,
        column: col,
        width: width_cells,
        ..
    } = viewport;
    let style_at = |cell| {
        let colors =
            highlight_color_at_cell(highlight_layers, cell, normal_color.fg).or_else(|| {
                let (cells, _) = decorations.undo_redo_highlight?;
                (cells.get(cell) == Some(&true)).then(|| {
                    normal_color.with_colors(
                        normal_color.fg,
                        color_column
                            .filter(|(column, _)| *column == cell)
                            .map_or(normal_color.bg, |(_, background)| background),
                    )
                })
            })?;
        Some(decorations.apply(colors.into(), cell..cell + 1))
    };
    let mut cell = start_cell;
    while cell < width_cells {
        let Some(text_style) = style_at(cell) else {
            cell += 1;
            continue;
        };

        let run_start = cell;
        cell += 1;
        while cell < width_cells && style_at(cell) == Some(text_style) {
            cell += 1;
        }
        let spaces = " ".repeat(cell - run_start);
        window.write_str_styled(
            row,
            col.saturating_add(run_start as u16),
            &spaces,
            text_style,
        )?;
    }

    Ok(())
}

pub(crate) fn highlight_color_at_cell(
    highlight_layers: &[(&[bool], Color, Option<Color>)],
    cell: usize,
    foreground: Color,
) -> Option<TextStyle> {
    highlight_layers
        .iter()
        .find_map(|(cells, background, override_foreground)| {
            cells
                .get(cell)
                .copied()
                .unwrap_or(false)
                .then_some(TextStyle::new(
                    override_foreground.unwrap_or(foreground),
                    *background,
                ))
        })
}

pub(crate) fn draw_plain_line(
    window: &mut dyn Window,
    viewport: LineViewport,
    source_line: &str,
    default_colors: TextStyle,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
    let LineViewport {
        row,
        column: col,
        scroll_x,
        width: width_cells,
    } = viewport;

    if width_cells == 0 {
        return Ok(());
    }

    let mut used_cells = 0usize;
    let mut line_cells = 0usize;

    for g in source_line.graphemes(true) {
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        line_cells = end_cell;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell < scroll_x {
            continue;
        }
        let visible_start = start_cell.saturating_sub(scroll_x);
        let visible_end = end_cell.saturating_sub(scroll_x);
        if visible_end > width_cells {
            break;
        }

        let colors = apply_color_column(
            default_colors.colors(),
            color_column,
            visible_start,
            visible_end,
        );
        let text_style = minui::Style::from(default_colors).with_colors(colors);
        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_styled(
                row,
                col.saturating_add(visible_start as u16),
                &spaces,
                text_style,
            )?;
        } else {
            window.write_str_styled(
                row,
                col.saturating_add(visible_start as u16),
                g,
                text_style,
            )?;
        }
        used_cells = visible_end;
    }

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
    {
        window.write_str_styled(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            (default_colors.with_colors(default_colors.fg, bg)).into(),
        )?;
    }

    Ok(())
}

pub(crate) fn draw_visible_ascii_plain_line(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    visible_line: &str,
    width_cells: usize,
    default_colors: TextStyle,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    let visible_len = visible_line.len().min(width_cells);
    let Some((visible_col, bg)) = color_column else {
        if visible_len > 0 {
            window.write_str_styled(
                row,
                col,
                &visible_line[..visible_len],
                default_colors.into(),
            )?;
        }
        return Ok(());
    };

    if visible_col >= width_cells {
        if visible_len > 0 {
            window.write_str_styled(
                row,
                col,
                &visible_line[..visible_len],
                default_colors.into(),
            )?;
        }
        return Ok(());
    }

    if visible_col < visible_len {
        if visible_col > 0 {
            window.write_str_styled(
                row,
                col,
                &visible_line[..visible_col],
                default_colors.into(),
            )?;
        }

        let next = visible_col.saturating_add(1);
        window.write_str_styled(
            row,
            col.saturating_add(visible_col as u16),
            &visible_line[visible_col..next],
            (default_colors.with_colors(default_colors.fg, bg)).into(),
        )?;

        if next < visible_len {
            window.write_str_styled(
                row,
                col.saturating_add(next as u16),
                &visible_line[next..visible_len],
                default_colors.into(),
            )?;
        }
        return Ok(());
    }

    if visible_len > 0 {
        window.write_str_styled(
            row,
            col,
            &visible_line[..visible_len],
            default_colors.into(),
        )?;
    }
    window.write_str_styled(
        row,
        col.saturating_add(visible_col as u16),
        " ",
        (default_colors.with_colors(default_colors.fg, bg)).into(),
    )
}

pub(crate) fn visible_color_column(
    scroll_x: usize,
    text_w: usize,
    color_column: Option<usize>,
    bg: Color,
) -> Option<(usize, Color)> {
    let visible_col = color_column?.checked_sub(scroll_x)?;
    (visible_col < text_w).then_some((visible_col, bg))
}

pub(crate) fn draw_snippet_placeholders(
    window: &mut dyn Window,
    viewport: LineViewport,
    source_line: &str,
    ranges: &[std::ops::Range<usize>],
    style: &UiStyle,
    decorations: LineDecorations<'_>,
    syntax_spans: Option<&[crate::ui::syntax::LineSyntaxSpan]>,
) -> minui::Result<()> {
    let LineViewport {
        row,
        column: content_x,
        scroll_x,
        width: text_w,
    } = viewport;

    if ranges.is_empty() || text_w == 0 {
        return Ok(());
    }

    let visible_end = scroll_x.saturating_add(text_w);
    let color = style.snippet;
    let mut cell = 0usize;
    let mut char_col = 0usize;
    for (start_byte, grapheme) in source_line.grapheme_indices(true) {
        let width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        let start_cell = cell;
        let end_cell = cell.saturating_add(width);
        let start_col = char_col;
        let end_col = char_col.saturating_add(grapheme.chars().count());
        cell = end_cell;
        char_col = end_col;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell >= visible_end {
            break;
        }
        if start_cell < scroll_x || end_cell > visible_end {
            continue;
        }
        if !ranges
            .iter()
            .any(|range| start_col < range.end && end_col > range.start)
        {
            continue;
        }

        let visible_x = start_cell.saturating_sub(scroll_x);
        let base_style = syntax_style_for_range(
            color,
            style,
            syntax_spans.unwrap_or(&[]),
            start_byte,
            start_byte + grapheme.len(),
        )
        .with_colors(color.colors());
        let text_style = decorations.apply(base_style, visible_x..visible_x + width);
        if grapheme == "\t" {
            let spaces = " ".repeat(width.max(1));
            window.write_str_styled(
                row,
                content_x.saturating_add(visible_x as u16),
                &spaces,
                text_style,
            )?;
        } else {
            window.write_str_styled(
                row,
                content_x.saturating_add(visible_x as u16),
                grapheme,
                text_style,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn draw_inline_diagnostic(
    window: &mut dyn Window,
    viewport: LineViewport,
    source_line: &str,
    style: &UiStyle,
    diagnostic: &crate::app::DiagnosticLine,
) -> minui::Result<()> {
    draw_inline_diagnostic_shifted(window, viewport, source_line, style, diagnostic, 0)
}

pub(crate) fn clear_inline_diagnostic(
    window: &mut dyn Window,
    row: u16,
    content_x: u16,
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    style: &UiStyle,
) -> minui::Result<()> {
    if text_w == 0 {
        return Ok(());
    }

    let Some(start_cell) = inline_diagnostic_start_cell(source_line, scroll_x, text_w, 0) else {
        return Ok(());
    };
    let blank = " ".repeat(text_w.saturating_sub(start_cell));
    window.write_str_styled(
        row,
        content_x.saturating_add(start_cell as u16),
        &blank,
        style.editor_text.into(),
    )
}

pub(crate) fn draw_inline_diagnostic_shifted(
    window: &mut dyn Window,
    viewport: LineViewport,
    source_line: &str,
    style: &UiStyle,
    diagnostic: &crate::app::DiagnosticLine,
    shift_cells: usize,
) -> minui::Result<()> {
    let LineViewport {
        row,
        column: content_x,
        scroll_x,
        width: text_w,
    } = viewport;

    if text_w == 0 {
        return Ok(());
    }

    let Some(start_cell) = inline_diagnostic_start_cell(source_line, scroll_x, text_w, shift_cells)
    else {
        return Ok(());
    };
    let available = text_w.saturating_sub(start_cell);
    let inline_text = inline_diagnostic_text(
        &diagnostic.inline_text,
        diagnostic.severity,
        style.icons_enabled,
    );
    let inline_text = crate::ui::widgets::popup::clip_text_to_cells(&inline_text, available);
    if inline_text.is_empty() {
        return Ok(());
    }

    let colors = style.diagnostic_inline.colors(diagnostic.severity);

    window.write_str_styled(
        row,
        content_x.saturating_add(start_cell as u16),
        &inline_text,
        colors.into(),
    )
}

pub(crate) fn inline_diagnostic_text(
    text: &str,
    severity: crate::app::DiagnosticSeverity,
    icons_enabled: bool,
) -> Cow<'_, str> {
    if !icons_enabled {
        return Cow::Borrowed(text);
    }
    let Some(message) = text.strip_prefix("▌ ") else {
        return Cow::Borrowed(text);
    };
    Cow::Owned(format!("▌ {} {message}", diagnostic_icon(severity)))
}

pub(crate) fn inline_diagnostic_start_cell(
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    shift_cells: usize,
) -> Option<usize> {
    let line_width = visible_content_cell_width(source_line, scroll_x, text_w);
    let start_cell = line_width.saturating_add(shift_cells).saturating_add(5);
    (start_cell < text_w).then_some(start_cell)
}
