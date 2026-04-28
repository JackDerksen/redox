use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use redox_core::{BufferId, EditorSession};

use minui::input::Clipboard;
use minui::prelude::{
    input::{Event, KeyKind},
    render::{Color, ColorPair, TabPolicy, TerminalWindow, Window, cell_width},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;

mod app;
mod input;
mod ui;

use app::{EditorState, FramePerfSample};
use input::{InputAction, map_event_with_context};

use crate::ui::helpers::apply_color_column;
use ui::overlays::{
    active_delimiter_highlights, active_scope_indent_guides, draw_delimiter_highlights,
    draw_indent_guides,
};
use ui::syntax::{
    VisibleLineSyntaxSpans, draw_line_with_syntax, lexical_fallback_line_spans,
    scope_guides_enabled, syntax_color_for_range,
};
use ui::{
    STATUS_BAR_HEIGHT_CELLS, TextViewport, UiStyle, about_popup_inner_size,
    build_editor_status_bar, draw_about_popup_view, draw_command_line_popup,
    draw_explorer_popup_view, draw_finder_popup, draw_perf_popup_view, draw_pin_selector_popup,
    draw_status_toast, explorer_popup_inner_size, language_for_path, perf_popup_layout,
    perf_popup_occludes_cursor, snapshot_lines_wrapped_cached, status_toast_occludes_cursor,
};

const GUTTER_CONTENT_PADDING: u16 = 1;
const COLOR_COLUMN: usize = 79;
const TARGET_FRAME_RATE_HZ: u64 = 60;
const TARGET_FRAME_BUDGET: Duration = Duration::from_nanos(1_000_000_000 / TARGET_FRAME_RATE_HZ);

enum LaunchTarget {
    Empty,
    File(PathBuf),
    Explorer(PathBuf),
}

fn draw_buffer_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    perf: &mut FramePerfSample,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let popup_overlay_active = matches!(
        state.mode,
        app::EditorMode::Command
            | app::EditorMode::Search
            | app::EditorMode::Finder
            | app::EditorMode::PinSelect
    ) || state.explorer_popup().is_some()
        || state.about_popup().is_some();
    let background_style = if popup_overlay_active {
        style.dimmed()
    } else {
        style
    };
    let editor_text = ColorPair::new(background_style.theme.white, background_style.theme.bg);
    fill_background(window, vw, vh, editor_text)?;
    let status_h: u16 = STATUS_BAR_HEIGHT_CELLS;
    let text_h = vh.saturating_sub(status_h);
    let load_start = Instant::now();
    state.pump_active_loading(text_h as usize);
    perf.load += load_start.elapsed();

    if let Some(popup) = state.explorer_popup() {
        if let Some(background_id) = state.explorer_background_buffer_id()
            && !state.explorer_background_is_placeholder_blank()
        {
            draw_buffer_snapshot_for_id(
                state,
                background_style,
                background_id,
                vw,
                text_h,
                editor_text,
                window,
            )?;
        }
        let (inner_w, inner_h) = explorer_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        let cursor_spec = draw_explorer_popup_view(state, style, window, popup)?;
        let toast_layout = draw_status_toast(state, style, window)?;
        if let Some(cursor) = cursor_spec {
            let cursor_hidden_by_toast = toast_layout
                .is_some_and(|layout| status_toast_occludes_cursor(layout, cursor.x, cursor.y));
            if cursor_hidden_by_toast {
                hide_cursor(window);
            } else {
                window.request_cursor(cursor);
            }
        }
        return Ok(());
    }

    if let Some(popup) = state.about_popup() {
        if let Some(background_id) = state.about_background_buffer_id() {
            draw_buffer_snapshot_for_id(
                state,
                background_style,
                background_id,
                vw,
                text_h,
                editor_text,
                window,
            )?;
        }
        let (inner_w, inner_h) = about_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        draw_about_popup_view(state, style, window, popup)?;
        hide_cursor(window);
        return Ok(());
    }

    let active_cursor_line = state.active_cursor_pos().line;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let gutter_w = line_number_gutter_width(total_lines);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let text_w = vw.saturating_sub(content_x);
    state.set_viewport_size(
        text_w as usize,
        text_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );
    state.ensure_rain_animation(text_w, text_h, editor_text, background_style);

    if let Some(animation) = state.active_rain_animation() {
        draw_relative_line_numbers(
            window,
            background_style,
            gutter_w,
            text_h,
            animation.first_line(),
            active_cursor_line,
            total_lines,
        )?;
        draw_gutter_padding(
            window,
            background_style,
            gutter_w,
            text_h,
            GUTTER_CONTENT_PADDING,
        )?;
        animation.draw(window, 0, content_x, text_w as usize, text_h as usize)?;

        let status = build_editor_status_bar(state, style);
        status.draw(window)?;
        if matches!(
            state.mode,
            app::EditorMode::Command | app::EditorMode::Search
        ) {
            draw_command_line_popup(state, style, window)?;
            return Ok(());
        }
        hide_cursor(window);
        return Ok(());
    }

    let visual_selection = state.active_visual_selection();
    let one_shot_highlight = state.one_shot_highlight();
    let syntax_language = language_for_path(state.session.active_meta().path.as_deref());
    let (snapshot, spec, scroll_x, snapshot_time) =
        state.with_active_buffer_view_mut(|buffer, view| {
            let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_w,
                height: text_h,
            };
            let snapshot_start = Instant::now();
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
            let spec = view
                .cursor
                .cursor_spec(buffer, text_w as usize, text_h as usize);
            let snapshot_time = snapshot_start.elapsed();
            (snapshot, spec, scroll_x, snapshot_time)
        });
    perf.snapshot += snapshot_time;
    let overlay_start = Instant::now();
    let search_highlights =
        state.active_search_highlight_ranges(snapshot.first_line, snapshot.lines.len());
    perf.overlays += overlay_start.elapsed();

    draw_relative_line_numbers(
        window,
        background_style,
        gutter_w,
        text_h,
        snapshot.first_line,
        active_cursor_line,
        total_lines,
    )?;
    draw_gutter_padding(
        window,
        background_style,
        gutter_w,
        text_h,
        GUTTER_CONTENT_PADDING,
    )?;

    let (syntax_time, overlay_time, lines_time) =
        state.with_active_buffer_view_mut(|buffer, view| {
            let cursor = view.cursor.cursor;
            let syntax_start = Instant::now();
            let analysis_version = view.analysis_version();
            let scope_guides_enabled = scope_guides_enabled(syntax_language);
            let tree_sitter_scope = scope_guides_enabled
                .then(|| {
                    view.syntax_highlighter
                        .active_scope_pair_for_display_cached(
                            buffer,
                            syntax_language,
                            analysis_version,
                            cursor,
                        )
                })
                .flatten();
            let syntax_spans = view.syntax_highlighter.visible_line_spans_cached(
                syntax_language,
                snapshot.first_line,
                snapshot.lines.len(),
            );
            let syntax_time = syntax_start.elapsed();
            let overlay_start = Instant::now();
            let delimiter_analysis = view.delimiter_pair_cache.get_for_display();
            let delimiter_highlights = delimiter_analysis
                .map(|analysis| {
                    active_delimiter_highlights(
                        buffer,
                        cursor,
                        snapshot.first_line,
                        snapshot.lines.len(),
                        analysis,
                    )
                })
                .unwrap_or_default();
            let active_scope_guides = if scope_guides_enabled {
                active_scope_indent_guides(
                    tree_sitter_scope,
                    buffer,
                    cursor,
                    snapshot.first_line,
                    snapshot.lines.len(),
                    scroll_x,
                    text_w as usize,
                    delimiter_analysis,
                )
            } else {
                BTreeMap::new()
            };
            let overlay_time = overlay_start.elapsed();

            let lines_start = Instant::now();
            draw_snapshot_lines(
                window,
                buffer,
                &snapshot,
                content_x,
                scroll_x,
                text_w as usize,
                editor_text,
                background_style,
                syntax_spans,
                &delimiter_highlights,
                &active_scope_guides,
                &search_highlights,
                visual_selection,
                one_shot_highlight,
                syntax_language.is_none(),
            )?;
            let lines_time = lines_start.elapsed();
            Ok::<_, minui::Error>((syntax_time, overlay_time, lines_time))
        })?;
    perf.syntax += syntax_time;
    perf.overlays += overlay_time;
    perf.lines += lines_time;
    state.advance_one_shot_highlight();

    // --- Status bar (bottom row) ---
    let status_start = Instant::now();
    let status = build_editor_status_bar(state, style);

    status.draw(window)?;
    perf.status += status_start.elapsed();

    let perf_popup_layout = if let Some(popup) = state.perf_popup()
        && !matches!(
            state.mode,
            app::EditorMode::Command
                | app::EditorMode::Search
                | app::EditorMode::Finder
                | app::EditorMode::PinSelect
        ) {
        let layout = perf_popup_layout(vw, vh, style);
        draw_perf_popup_view(style, window, popup)?;
        Some(layout)
    } else {
        None
    };

    let mut cursor_spec = None;
    if matches!(
        state.mode,
        app::EditorMode::Command | app::EditorMode::Search
    ) {
        draw_command_line_popup(state, style, window)?;
    } else if let Some(popup) = state.finder_popup() {
        draw_finder_popup(&popup, style, window)?;
    } else if let Some(popup) = state.pin_selector_popup() {
        draw_pin_selector_popup(&popup, style, window)?;
    } else if spec.visible {
        cursor_spec = Some(minui::window::CursorSpec {
            x: spec.x.saturating_add(content_x),
            y: spec.y,
            visible: true,
        });
    }

    let toast_layout = draw_status_toast(state, style, window)?;

    if let Some(cursor) = cursor_spec {
        let cursor_hidden_by_perf = perf_popup_layout
            .is_some_and(|layout| perf_popup_occludes_cursor(layout, cursor.x, cursor.y));
        let cursor_hidden_by_toast = toast_layout
            .is_some_and(|layout| status_toast_occludes_cursor(layout, cursor.x, cursor.y));
        if cursor_hidden_by_perf || cursor_hidden_by_toast {
            hide_cursor(window);
        } else {
            window.request_cursor(cursor);
        }
    }

    Ok(())
}

fn draw_gutter_padding(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    padding_w: u16,
) -> minui::Result<()> {
    if padding_w == 0 || text_h == 0 {
        return Ok(());
    }

    let pad = " ".repeat(padding_w as usize);
    let color = ColorPair::new(style.theme.bg, style.theme.bg);
    for row in 0..text_h {
        window.write_str_colored(row, gutter_w, &pad, color)?;
    }
    Ok(())
}

fn line_number_gutter_width(total_lines: usize) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    // digits + separator column
    digits.saturating_add(1)
}

fn draw_relative_line_numbers(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    first_line: usize,
    cursor_line: usize,
    total_lines: usize,
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let sep_x = gutter_w.saturating_sub(1);
    let number_w = gutter_w.saturating_sub(1) as usize;
    let relative_color = ColorPair::new(style.theme.dark_gray, style.theme.bg);
    let current_color = ColorPair::new(style.theme.white, style.theme.bg);

    for row in 0..text_h {
        let line_idx = first_line.saturating_add(row as usize);
        if line_idx >= total_lines {
            continue;
        }

        let num = if line_idx == cursor_line {
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

        let color = if line_idx == cursor_line {
            current_color
        } else {
            relative_color
        };

        if number_w > 0 {
            window.write_str_colored(row, 0, &text, color)?;
        }

        window.write_str_colored(row, sep_x, "▕", color)?;
    }

    Ok(())
}

fn draw_line_with_highlights(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    normal_color: ColorPair,
    color_column: Option<(usize, Color)>,
    style: UiStyle,
    syntax_spans: Option<&[ui::syntax::LineSyntaxSpan]>,
    highlight_layers: &[(&[bool], Color)],
    highlight_empty_line: bool,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    if source_line.is_empty() {
        if highlight_empty_line {
            let bg = highlight_layers
                .first()
                .map(|(_, bg)| *bg)
                .unwrap_or(normal_color.bg);
            window.write_str_colored(row, col, " ", ColorPair::new(normal_color.fg, bg))?;
            if let Some((visible_col, bg)) = color_column
                && visible_col < width_cells
                && visible_col != 0
            {
                window.write_str_colored(
                    row,
                    col.saturating_add(visible_col as u16),
                    " ",
                    ColorPair::new(normal_color.fg, bg),
                )?;
            }
        } else if let Some((visible_col, bg)) = color_column
            && visible_col < width_cells
        {
            window.write_str_colored(
                row,
                col.saturating_add(visible_col as u16),
                " ",
                ColorPair::new(normal_color.fg, bg),
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
        if used_cells.saturating_add(g_width) > width_cells {
            break;
        }

        let visible_start = start_cell.max(scroll_x).saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        let base_color = syntax_spans
            .map(|spans| syntax_color_for_range(normal_color, style, spans, start_byte, end_byte))
            .unwrap_or(normal_color);
        let highlight_bg = highlight_layers.iter().find_map(|(cells, bg)| {
            (visible_start < visible_end
                && visible_end <= cells.len()
                && cells[visible_start..visible_end]
                    .iter()
                    .any(|selected| *selected))
            .then_some(*bg)
        });
        let color = if let Some(bg) = highlight_bg {
            ColorPair::new(base_color.fg, bg)
        } else {
            apply_color_column(base_color, color_column, start_cell, end_cell)
        };

        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_colored(row, col.saturating_add(used_cells as u16), &spaces, color)?;
        } else {
            window.write_str_colored(row, col.saturating_add(used_cells as u16), g, color)?;
        }
        used_cells = used_cells.saturating_add(g_width);
    }

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
    {
        window.write_str_colored(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            ColorPair::new(normal_color.fg, bg),
        )?;
    }

    Ok(())
}

fn fill_background(
    window: &mut dyn Window,
    width: u16,
    height: u16,
    colors: ColorPair,
) -> minui::Result<()> {
    if width == 0 || height == 0 {
        return Ok(());
    }

    let row = " ".repeat(width as usize);
    for y in 0..height {
        window.write_str_colored(y, 0, &row, colors)?;
    }
    Ok(())
}

fn hide_cursor(window: &mut dyn Window) {
    window.request_cursor(minui::window::CursorSpec {
        x: 0,
        y: 0,
        visible: false,
    });
}

fn draw_buffer_snapshot_for_id(
    state: &mut EditorState,
    style: UiStyle,
    buffer_id: BufferId,
    width: u16,
    height: u16,
    colors: ColorPair,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let syntax_language = state
        .session
        .meta(buffer_id)
        .and_then(|meta| language_for_path(meta.path.as_deref()));
    let Some(result) = state.with_buffer_view_mut(buffer_id, |buffer, view| {
        let cursor = view.cursor.cursor;
        let total_lines = buffer.len_lines().max(1);
        let gutter_w = line_number_gutter_width(total_lines);
        let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
        let text_w = width.saturating_sub(content_x);
        let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
        let viewport = TextViewport {
            scroll_x,
            scroll_y,
            width: text_w,
            height,
        };
        let snapshot = snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
        let analysis_version = view.analysis_version();
        let scope_guides_enabled = scope_guides_enabled(syntax_language);
        let tree_sitter_scope = scope_guides_enabled
            .then(|| {
                view.syntax_highlighter
                    .active_scope_pair_for_display_cached(
                        buffer,
                        syntax_language,
                        analysis_version,
                        cursor,
                    )
            })
            .flatten();
        let syntax_spans = view.syntax_highlighter.visible_line_spans_cached(
            syntax_language,
            snapshot.first_line,
            snapshot.lines.len(),
        );
        let delimiter_analysis = view.delimiter_pair_cache.get_for_display();
        let delimiter_highlights = delimiter_analysis
            .map(|analysis| {
                active_delimiter_highlights(
                    buffer,
                    cursor,
                    snapshot.first_line,
                    snapshot.lines.len(),
                    analysis,
                )
            })
            .unwrap_or_default();
        let active_scope_guides = if scope_guides_enabled {
            active_scope_indent_guides(
                tree_sitter_scope,
                buffer,
                cursor,
                snapshot.first_line,
                snapshot.lines.len(),
                scroll_x,
                width.saturating_sub(content_x) as usize,
                delimiter_analysis,
            )
        } else {
            BTreeMap::new()
        };

        draw_relative_line_numbers(
            window,
            style,
            gutter_w,
            height,
            snapshot.first_line,
            view.cursor.cursor.line,
            total_lines,
        )?;
        draw_gutter_padding(window, style, gutter_w, height, GUTTER_CONTENT_PADDING)?;

        let search_highlights = BTreeMap::new();
        draw_snapshot_lines(
            window,
            buffer,
            &snapshot,
            content_x,
            scroll_x,
            width.saturating_sub(content_x) as usize,
            colors,
            style,
            syntax_spans,
            &delimiter_highlights,
            &active_scope_guides,
            &search_highlights,
            None,
            None,
            syntax_language.is_none(),
        )
    }) else {
        return Ok(());
    };
    result
}

fn draw_snapshot_lines(
    window: &mut dyn Window,
    buffer: &redox_core::TextBuffer,
    snapshot: &ui::render::RenderSnapshot,
    content_x: u16,
    scroll_x: usize,
    text_w: usize,
    default_colors: ColorPair,
    style: UiStyle,
    syntax_spans: Option<VisibleLineSyntaxSpans<'_>>,
    delimiter_highlights: &BTreeMap<usize, Vec<usize>>,
    active_scope_guides: &BTreeMap<usize, Vec<usize>>,
    search_highlights: &BTreeMap<usize, Vec<std::ops::Range<usize>>>,
    visual_selection: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    one_shot_highlight: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    lexical_fallback_enabled: bool,
) -> minui::Result<()> {
    let color_column = visible_color_column(scroll_x, text_w, style.theme.color_column);
    for row in 0..snapshot.lines.len() {
        let line_idx = snapshot.first_line + row;
        let visible_line = snapshot.lines.get(row).map(String::as_str).unwrap_or("");
        let source_line = buffer.line_string(line_idx);
        let fallback_line_spans = lexical_fallback_enabled
            .then(|| lexical_fallback_line_spans(&source_line))
            .filter(|spans| !spans.is_empty());
        let syntax_line_spans = syntax_spans
            .and_then(|rows| rows.get(row))
            .or(fallback_line_spans.as_deref());
        let highlighted_chars = delimiter_highlights
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let visible_indent_guides = active_scope_guides
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let has_search_highlights = search_highlights
            .get(&line_idx)
            .is_some_and(|ranges| !ranges.is_empty());
        let transient_selection = visual_selection
            .map(|(selection, mode)| (selection, mode, style.theme.selection_bg))
            .or_else(|| {
                one_shot_highlight
                    .map(|(selection, mode)| (selection, mode, style.theme.light_purple))
            });

        if transient_selection.is_none()
            && !has_search_highlights
            && syntax_line_spans.is_none_or(|spans| spans.is_empty())
            && highlighted_chars.is_empty()
            && visible_indent_guides.is_empty()
            && visible_line.is_ascii()
            && !visible_line.contains('\t')
        {
            draw_visible_ascii_plain_line(
                window,
                row as u16,
                content_x,
                visible_line,
                text_w,
                default_colors,
                color_column,
            )?;
            continue;
        }

        let occupied_text_cells = if visible_indent_guides.is_empty() {
            Vec::new()
        } else {
            occupied_visible_cells(&source_line, scroll_x, text_w)
        };
        let search_cells = search_highlights
            .get(&line_idx)
            .map(|ranges| highlighted_visible_cells(&source_line, scroll_x, text_w, ranges));
        if let Some((selection, mode, selection_bg)) = transient_selection {
            let highlight_empty_line = buffer.line_len_chars(line_idx) == 0
                && selected_empty_line(selection, mode, line_idx);
            if let Some(sel_range) =
                buffer.visual_selection_char_range_on_line(selection, mode, line_idx)
            {
                let selected_cells = selected_visible_cells(
                    &source_line,
                    scroll_x,
                    text_w,
                    sel_range.start,
                    sel_range.end,
                );
                let highlight_layers = if let Some(search_cells) = search_cells.as_ref() {
                    vec![
                        (selected_cells.as_slice(), selection_bg),
                        (search_cells.as_slice(), style.theme.selection_bg),
                    ]
                } else {
                    vec![(selected_cells.as_slice(), selection_bg)]
                };
                draw_line_with_highlights(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    default_colors,
                    color_column,
                    style,
                    syntax_line_spans,
                    &highlight_layers,
                    highlight_empty_line,
                )?;
                draw_indent_guides(
                    window,
                    row as u16,
                    content_x,
                    visible_indent_guides,
                    &occupied_text_cells,
                    style,
                    Some((&selected_cells, selection_bg)),
                )?;
                draw_delimiter_highlights(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    highlighted_chars,
                    style,
                )?;
                continue;
            }
            if highlight_empty_line {
                let highlight_layers = [(&[][..], selection_bg)];
                draw_line_with_highlights(
                    window,
                    row as u16,
                    content_x,
                    "",
                    scroll_x,
                    text_w,
                    default_colors,
                    color_column,
                    style,
                    syntax_line_spans,
                    &highlight_layers,
                    true,
                )?;
                continue;
            }
        }

        if let Some(search_cells) = search_cells.as_ref()
            && search_cells.iter().any(|selected| *selected)
        {
            let highlight_layers = [(search_cells.as_slice(), style.theme.selection_bg)];
            draw_line_with_highlights(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                default_colors,
                color_column,
                style,
                syntax_line_spans,
                &highlight_layers,
                false,
            )?;
            draw_indent_guides(
                window,
                row as u16,
                content_x,
                visible_indent_guides,
                &occupied_text_cells,
                style,
                None,
            )?;
            draw_delimiter_highlights(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                highlighted_chars,
                style,
            )?;
            continue;
        }

        if let Some(spans) = syntax_line_spans
            && !spans.is_empty()
        {
            draw_line_with_syntax(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                default_colors,
                color_column,
                style,
                spans,
            )?;
            draw_indent_guides(
                window,
                row as u16,
                content_x,
                visible_indent_guides,
                &occupied_text_cells,
                style,
                None,
            )?;
            draw_delimiter_highlights(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                highlighted_chars,
                style,
            )?;
            continue;
        }

        draw_plain_line(
            window,
            row as u16,
            content_x,
            &source_line,
            scroll_x,
            text_w,
            default_colors,
            color_column,
        )?;
        draw_indent_guides(
            window,
            row as u16,
            content_x,
            visible_indent_guides,
            &occupied_text_cells,
            style,
            None,
        )?;
        draw_delimiter_highlights(
            window,
            row as u16,
            content_x,
            &source_line,
            scroll_x,
            text_w,
            highlighted_chars,
            style,
        )?;
    }

    Ok(())
}

fn selected_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    sel_start_char: usize,
    sel_end_char_exclusive: usize,
) -> Vec<bool> {
    highlighted_visible_cells(
        source_line,
        scroll_x,
        width_cells,
        &[sel_start_char..sel_end_char_exclusive],
    )
}

fn highlighted_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    ranges: &[std::ops::Range<usize>],
) -> Vec<bool> {
    let mut selected = vec![false; width_cells];
    if width_cells == 0 || source_line.is_empty() || ranges.is_empty() {
        return selected;
    }

    let mut line_cells = 0usize;
    let mut char_idx = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

    for g in source_line.graphemes(true) {
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let g_chars = g.chars().count();
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_char = char_idx;
        let end_char = char_idx.saturating_add(g_chars);

        line_cells = end_cell;
        char_idx = end_char;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell >= max_visible_cell {
            break;
        }
        if !ranges
            .iter()
            .any(|range| start_char < range.end && end_char > range.start)
        {
            continue;
        }

        let visible_start = start_cell.max(scroll_x).saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        for cell in visible_start..visible_end {
            selected[cell] = true;
        }
    }

    selected
}

fn occupied_visible_cells(source_line: &str, scroll_x: usize, width_cells: usize) -> Vec<bool> {
    let mut occupied = vec![false; width_cells];
    if width_cells == 0 || source_line.is_empty() {
        return occupied;
    }

    let mut line_cells = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

    for g in source_line.graphemes(true) {
        if g.chars().all(char::is_whitespace) {
            let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
            line_cells = line_cells.saturating_add(g_width.max(1));
            continue;
        }

        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width.max(1));
        line_cells = end_cell;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell >= max_visible_cell {
            break;
        }

        let visible_start = start_cell.saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        for cell in visible_start..visible_end.min(occupied.len()) {
            occupied[cell] = true;
        }
    }

    occupied
}

fn selected_empty_line(
    selection: redox_core::Selection,
    mode: redox_core::VisualModeKind,
    line_idx: usize,
) -> bool {
    matches!(mode, redox_core::VisualModeKind::Line) && {
        let (start, end) = selection.ordered();
        line_idx >= start.line && line_idx <= end.line
    }
}

fn draw_plain_line(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    default_colors: ColorPair,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
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
        if used_cells.saturating_add(g_width) > width_cells {
            break;
        }

        let colors = apply_color_column(default_colors, color_column, start_cell, end_cell);
        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_colored(
                row,
                col.saturating_add(used_cells as u16),
                &spaces,
                colors,
            )?;
        } else {
            window.write_str_colored(row, col.saturating_add(used_cells as u16), g, colors)?;
        }
        used_cells = used_cells.saturating_add(g_width);
    }

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
    {
        window.write_str_colored(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            ColorPair::new(default_colors.fg, bg),
        )?;
    }

    Ok(())
}

fn draw_visible_ascii_plain_line(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    visible_line: &str,
    width_cells: usize,
    default_colors: ColorPair,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    let visible_len = visible_line.len().min(width_cells);
    let Some((visible_col, bg)) = color_column else {
        if visible_len > 0 {
            window.write_str_colored(row, col, &visible_line[..visible_len], default_colors)?;
        }
        return Ok(());
    };

    if visible_col >= width_cells {
        if visible_len > 0 {
            window.write_str_colored(row, col, &visible_line[..visible_len], default_colors)?;
        }
        return Ok(());
    }

    if visible_col < visible_len {
        if visible_col > 0 {
            window.write_str_colored(row, col, &visible_line[..visible_col], default_colors)?;
        }

        let next = visible_col.saturating_add(1);
        window.write_str_colored(
            row,
            col.saturating_add(visible_col as u16),
            &visible_line[visible_col..next],
            ColorPair::new(default_colors.fg, bg),
        )?;

        if next < visible_len {
            window.write_str_colored(
                row,
                col.saturating_add(next as u16),
                &visible_line[next..visible_len],
                default_colors,
            )?;
        }
        return Ok(());
    }

    if visible_len > 0 {
        window.write_str_colored(row, col, &visible_line[..visible_len], default_colors)?;
    }
    window.write_str_colored(
        row,
        col.saturating_add(visible_col as u16),
        " ",
        ColorPair::new(default_colors.fg, bg),
    )
}

fn visible_color_column(scroll_x: usize, text_w: usize, bg: Color) -> Option<(usize, Color)> {
    if COLOR_COLUMN < scroll_x {
        return None;
    }
    let visible_col = COLOR_COLUMN - scroll_x;
    (visible_col < text_w).then_some((visible_col, bg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minui::{ColorPair, Window};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn target_frame_budget_matches_sixty_fps() {
        assert_eq!(TARGET_FRAME_RATE_HZ, 60);
        assert_eq!(TARGET_FRAME_BUDGET, Duration::from_nanos(16_666_666));
    }

    fn temp_dir_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_lib_test_{tag}_{nanos}"))
    }

    struct TestWindow {
        width: u16,
        height: u16,
        cells: Vec<Vec<char>>,
    }

    impl TestWindow {
        fn new(width: u16, height: u16) -> Self {
            Self {
                width,
                height,
                cells: vec![vec![' '; width as usize]; height as usize],
            }
        }

        fn row_text(&self, row: u16) -> String {
            self.cells[row as usize].iter().collect()
        }

        fn write_text(&mut self, y: u16, x: u16, s: &str) {
            if y >= self.height {
                return;
            }

            let row = &mut self.cells[y as usize];
            for (idx, ch) in s.chars().enumerate() {
                let cell_x = x as usize + idx;
                if cell_x >= row.len() {
                    break;
                }
                row[cell_x] = ch;
            }
        }
    }

    impl Window for TestWindow {
        fn write_str(&mut self, y: u16, x: u16, s: &str) -> minui::Result<()> {
            self.write_text(y, x, s);
            Ok(())
        }

        fn write_str_colored(
            &mut self,
            y: u16,
            x: u16,
            s: &str,
            _colors: ColorPair,
        ) -> minui::Result<()> {
            self.write_text(y, x, s);
            Ok(())
        }

        fn flush(&mut self) -> minui::Result<()> {
            Ok(())
        }

        fn set_cursor_position(&mut self, _x: u16, _y: u16) -> minui::Result<()> {
            Ok(())
        }

        fn show_cursor(&mut self, _show: bool) -> minui::Result<()> {
            Ok(())
        }

        fn get_size(&self) -> (u16, u16) {
            (self.width, self.height)
        }

        fn clear_screen(&mut self) -> minui::Result<()> {
            for row in &mut self.cells {
                row.fill(' ');
            }
            Ok(())
        }

        fn clear_line(&mut self, y: u16) -> minui::Result<()> {
            if let Some(row) = self.cells.get_mut(y as usize) {
                row.fill(' ');
            }
            Ok(())
        }

        fn clear_area(&mut self, y1: u16, x1: u16, y2: u16, x2: u16) -> minui::Result<()> {
            let row_start = usize::from(y1.min(y2));
            let row_end = usize::from(y1.max(y2)).min(self.cells.len().saturating_sub(1));
            let col_start = usize::from(x1.min(x2));
            let col_end = usize::from(x1.max(x2)).min(self.width.saturating_sub(1) as usize);

            for row in row_start..=row_end {
                for col in col_start..=col_end {
                    self.cells[row][col] = ' ';
                }
            }

            Ok(())
        }
    }

    #[test]
    fn draw_snapshot_lines_renders_scrolled_plain_text_without_double_scroll() {
        let buffer = redox_core::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz\n");
        let snapshot = ui::render::RenderSnapshot::new(0, vec!["ijklmnop".to_string()]);
        let mut window = TestWindow::new(8, 1);
        let style = UiStyle::default();
        let default_colors = ColorPair::new(style.theme.white, style.theme.bg);

        draw_snapshot_lines(
            &mut window,
            &buffer,
            &snapshot,
            0,
            8,
            8,
            default_colors,
            style,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            None,
            false,
        )
        .expect("plain snapshot draw should succeed");

        assert_eq!(window.row_text(0), "ijklmnop");
    }

    #[test]
    fn draw_buffer_view_shows_status_toast_while_explorer_is_open() {
        let _lock = app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir_path("explorer_toast");
        fs::create_dir(&dir).expect("failed to create temp dir");
        fs::write(dir.join("alpha.txt"), "alpha").expect("failed to write fixture");

        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state
            .open_explorer_at_path(dir.clone())
            .expect("failed to open explorer");
        state.set_status("explorer write failed: invalid file name");

        let mut window = TestWindow::new(80, 24);
        let mut perf = FramePerfSample::default();
        draw_buffer_view(&mut state, UiStyle::default(), &mut window, &mut perf)
            .expect("draw should succeed");

        let screen = (0..24)
            .map(|row| window.row_text(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("explorer write failed"));

        let _ = fs::remove_dir_all(dir);
    }
}

fn parse_path_arg() -> anyhow::Result<LaunchTarget> {
    let mut args = env::args().skip(1);
    let Some(raw) = args.next() else {
        return Ok(LaunchTarget::Empty);
    };
    let path = PathBuf::from(&raw);
    if path.is_dir() {
        return Ok(LaunchTarget::Explorer(path));
    }
    Ok(LaunchTarget::File(path))
}

fn is_cancel_event(event: &Event) -> bool {
    matches!(event, Event::Escape)
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if matches!(key.key, KeyKind::Escape)
                    && !key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
        )
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
                    && matches!(key.key, KeyKind::Char('c') | KeyKind::Char('C'))
        )
}

fn handle_editor_event(
    state: &mut EditorState,
    clipboard: &mut Option<Clipboard>,
    event: Event,
) -> bool {
    if state.rain_is_active() {
        if is_cancel_event(&event) {
            state.stop_rain_animation();
        }
        return !state.should_quit;
    }

    if is_cancel_event(&event) && state.handle_normal_mode_escape_on_surface() {
        return !state.should_quit;
    }

    if is_cancel_event(&event) && state.dismiss_perf_popup() {
        return !state.should_quit;
    }

    let confirm_explorer_delete = state.has_pending_explorer_delete_confirmation();
    let action = match &event {
        Event::Paste(text) => InputAction::Paste(text.clone()),
        _ => map_event_with_context(
            &mut state.input,
            state.mode.as_input_mode(),
            confirm_explorer_delete,
            &event,
        ),
    };

    let (w, h) = state.viewport_size();
    match action {
        InputAction::PasteSystemClipboard => match clipboard.as_mut() {
            Some(system_clipboard) => match system_clipboard.paste() {
                Ok(text) => state.apply_input(InputAction::PasteSystemClipboardText(text), w, h),
                Err(e) => state.set_status(format!("clipboard paste failed: {e}")),
            },
            None => state.set_status("system clipboard unavailable"),
        },
        action => state.apply_input(action, w, h),
    }
    if let Some(text) = state.take_pending_system_clipboard() {
        match clipboard.as_mut() {
            Some(system_clipboard) => {
                if let Err(e) = system_clipboard.copy(&text) {
                    state.set_status(format!("clipboard copy failed: {e}"));
                } else {
                    state.set_status("yanked to system clipboard");
                }
            }
            None => {
                state.set_status("system clipboard unavailable");
            }
        }
    }

    !state.should_quit
}

pub fn run() -> minui::Result<()> {
    let launch = parse_path_arg().expect("failed to parse launch target");
    let launch_empty = matches!(&launch, LaunchTarget::Empty);
    let launch_explorer_dir = match &launch {
        LaunchTarget::Explorer(dir) => Some(dir.clone()),
        LaunchTarget::Empty | LaunchTarget::File(_) => None,
    };
    let session = match launch {
        LaunchTarget::Empty => {
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session")
        }
        LaunchTarget::File(path) => {
            EditorSession::open_initial_file(path).expect("failed to open initial file")
        }
        LaunchTarget::Explorer(_) => {
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session")
        }
    };

    let mut state = EditorState::new(session);
    if let Some(dir_path) = launch_explorer_dir {
        state
            .open_explorer_at_path(dir_path)
            .expect("failed to open explorer directory");
    }
    if launch_empty {
        state.command_open_about();
    }

    let mut window = TerminalWindow::new()?;
    window.set_auto_flush(false);
    let mut clipboard = Clipboard::new().ok();
    let style = UiStyle::default();

    const MAX_EVENTS_PER_FRAME: usize = 256;

    let mut pending_wake_event: Option<Event> = None;

    loop {
        let frame_start = Instant::now();
        let mut perf_sample = FramePerfSample::default();
        let input_start = Instant::now();
        let mut event_count = 0usize;

        if let Some(event) = pending_wake_event.take() {
            event_count += 1;
            if !handle_editor_event(&mut state, &mut clipboard, event) {
                return Ok(());
            }
        }

        for _ in 0..MAX_EVENTS_PER_FRAME {
            match window.poll_input()? {
                Some(event) => {
                    event_count += 1;
                    if !handle_editor_event(&mut state, &mut clipboard, event) {
                        return Ok(());
                    }
                }
                None => break,
            }
        }
        perf_sample.input = input_start.elapsed();
        perf_sample.event_count = event_count;
        state.poll_analysis_results();
        state.poll_finder_results();
        state.expire_status_message(Instant::now());

        if state.rain_is_active() {
            state.advance_rain_animation();
        }

        window.clear_cursor_request();
        let (w, h) = window.get_size();
        state.set_viewport_size(w as usize, h as usize);
        window.clear_screen()?;
        draw_buffer_view(&mut state, style, &mut window, &mut perf_sample)?;
        let flush_start = Instant::now();
        window.end_frame()?;
        perf_sample.flush = flush_start.elapsed();
        perf_sample.frame = frame_start.elapsed();
        state.record_perf_sample(perf_sample);

        if state.should_quit {
            return Ok(());
        }

        let remaining = TARGET_FRAME_BUDGET.saturating_sub(frame_start.elapsed());
        if !remaining.is_zero() {
            let event = window.get_input_timeout(remaining)?;
            if !matches!(event, Event::Unknown) {
                pending_wake_event = Some(event);
            }
        }
    }
}
