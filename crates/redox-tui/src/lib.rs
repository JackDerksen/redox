use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use redox_core::{BufferId, EditorSession};

use minui::KeybindAction;
use minui::input::Clipboard;
use minui::prelude::{
    input::{Event, KeyKind},
    render::{Color, ColorPair, TabPolicy, TerminalWindow, Window, cell_width},
    widgets::Widget,
};
use minui::widgets::WindowView;
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
    merge_line_spans_for_display, scope_guides_enabled, syntax_color_for_range,
};
use ui::widgets::popup::popup_occludes_cursor;
use ui::{
    STATUS_BAR_HEIGHT_CELLS, TextViewport, UiStyle, about_popup_inner_size,
    build_editor_status_bar, draw_about_popup_view, draw_code_actions_popup,
    draw_command_line_popup, draw_completion_popup, draw_completion_preview,
    draw_diagnostics_popup, draw_explorer_popup_view, draw_finder_popup,
    draw_lsp_marketplace_popup, draw_perf_popup_view, draw_pin_selector_popup, draw_status_toast,
    draw_symbol_info_popup, explorer_popup_inner_size, language_for_path,
    lsp_marketplace_popup_inner_size, perf_popup_layout, snapshot_lines_wrapped_cached,
};

pub(crate) const SOFT_TAB_WIDTH: usize = 4;
pub(crate) const SOFT_TAB: &str = "    ";

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
    let popup_overlay_active = state.mode.has_popup_overlay()
        || state.explorer_popup().is_some()
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
    state.refresh_active_git_diff();

    if let Some(popup) = state.explorer_popup() {
        let fallback_id = state
            .explorer_background_buffer_id()
            .filter(|_| !state.explorer_background_is_placeholder_blank());
        draw_popup_background(
            state,
            background_style,
            window,
            vw,
            text_h,
            editor_text,
            fallback_id,
        )?;
        let (inner_w, inner_h) = explorer_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        let cursor_spec = draw_explorer_popup_view(state, style, window, popup)?;
        let toast_layout = draw_status_toast(state, style, window)?;
        if let Some(cursor) = cursor_spec {
            let cursor_hidden_by_toast = toast_layout
                .is_some_and(|layout| popup_occludes_cursor(layout, cursor.x, cursor.y));
            if cursor_hidden_by_toast {
                hide_cursor(window);
            } else {
                window.request_cursor(cursor);
            }
        }
        return Ok(());
    }

    if let Some(popup) = state.about_popup() {
        draw_popup_background(
            state,
            background_style,
            window,
            vw,
            text_h,
            editor_text,
            state.about_background_buffer_id(),
        )?;
        let (inner_w, inner_h) = about_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        draw_about_popup_view(state, style, window, popup)?;
        hide_cursor(window);
        return Ok(());
    }

    if let Some(popup) = state.lsp_marketplace_popup() {
        draw_modal_popup_background(
            state,
            style,
            background_style,
            window,
            vw,
            text_h,
            editor_text,
            Some(state.session.active_id()),
            lsp_marketplace_popup_inner_size(vw, vh, style),
        )?;
        draw_lsp_marketplace_popup(&popup, style, window)?;
        hide_cursor(window);
        return Ok(());
    }

    if let Some(popup) = state.diagnostics_popup() {
        draw_modal_popup_background(
            state,
            style,
            background_style,
            window,
            vw,
            text_h,
            editor_text,
            Some(state.session.active_id()),
            finder_popup_inner_size(vw, vh, style),
        )?;
        draw_diagnostics_popup(&popup, style, window)?;
        hide_cursor(window);
        return Ok(());
    }

    if let Some(popup) = state.code_actions_popup() {
        draw_modal_popup_background(
            state,
            style,
            background_style,
            window,
            vw,
            text_h,
            editor_text,
            Some(state.session.active_id()),
            finder_popup_inner_size(vw, vh, style),
        )?;
        draw_code_actions_popup(&popup, style, window)?;
        hide_cursor(window);
        return Ok(());
    }

    let active_cursor_line = state.active_cursor_pos().line;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let show_git_marker_column = git_marker_column_visible(state.active_git_diff());
    let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let text_w = vw.saturating_sub(content_x);
    state.set_editor_area_size(vw as usize, text_h as usize);
    state.set_viewport_size(
        text_w as usize,
        text_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );
    if state.panes().len() > 1 {
        state.sync_active_pane_view();
        if let Some(rect) = state
            .pane_rects(vw, text_h)
            .into_iter()
            .find(|rect| rect.pane_id == state.active_pane_id())
        {
            let total_lines = state.session.active_buffer().len_lines().max(1);
            let show_git_marker_column = git_marker_column_visible(state.active_git_diff());
            let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
            let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
            let pane_text_w = rect.width.saturating_sub(content_x);
            state.set_viewport_size(
                pane_text_w as usize,
                rect.height.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
            );
            state.ensure_rain_animation(pane_text_w, rect.height, editor_text, background_style);
        }
        let split_background_style = if popup_overlay_active {
            background_style
        } else {
            style
        };
        draw_split_editor_panes(
            state,
            split_background_style,
            window,
            vw,
            text_h,
            editor_text,
            !popup_overlay_active,
        )?;
        state.advance_one_shot_highlight();
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

        let mut cursor_spec: Option<minui::window::CursorSpec> = None;
        let mut force_hide_cursor = false;

        if matches!(
            state.mode,
            app::EditorMode::Command | app::EditorMode::Search
        ) {
            draw_command_line_popup(state, style, window)?;
        } else if let Some(popup) = state.finder_popup() {
            draw_finder_popup(&popup, style, window)?;
        } else if let Some(popup) = state.pin_selector_popup() {
            draw_pin_selector_popup(&popup, style, window)?;
        } else if let Some(popup) = state.completion_popup() {
            if let Some(context) = active_split_cursor_context(state, vw, text_h) {
                let cursor_x = context.x;
                if let Some(preview) = state.completion_preview() {
                    let preview_width =
                        clipped_cell_width(&preview.text, vw.saturating_sub(cursor_x) as usize);
                    let active_cursor_line = state.active_cursor_pos().line;
                    let inline_diagnostic = state
                        .active_diagnostic_lines(active_cursor_line, 1)
                        .remove(&active_cursor_line);
                    let source_line = inline_diagnostic.as_ref().map(|_| {
                        state
                            .session
                            .active_buffer()
                            .line_string(active_cursor_line)
                    });
                    if let Some(source_line) = source_line.as_deref() {
                        clear_inline_diagnostic(
                            window,
                            context.y,
                            context.content_x,
                            source_line,
                            context.scroll_x,
                            context.text_w as usize,
                            background_style,
                        )?;
                    }
                    draw_completion_preview(
                        window,
                        style,
                        cursor_x,
                        context.y,
                        vw.saturating_sub(cursor_x),
                        &preview.text,
                        &preview.suffix,
                    )?;
                    if let Some((diagnostic, source_line)) =
                        inline_diagnostic.as_ref().zip(source_line.as_deref())
                    {
                        draw_inline_diagnostic_shifted(
                            window,
                            context.y,
                            context.content_x,
                            source_line,
                            context.scroll_x,
                            context.text_w as usize,
                            background_style,
                            diagnostic,
                            preview_width,
                        )?;
                    }
                }
                draw_completion_popup(&popup, style, window, cursor_x, context.y, context.height)?;
                cursor_spec = Some(minui::window::CursorSpec {
                    x: cursor_x,
                    y: context.y,
                    visible: true,
                });
            }
        } else if state.active_rain_animation().is_some() {
            force_hide_cursor = true;
        } else if let Some(cursor) = active_split_cursor(state, vw, text_h) {
            cursor_spec = Some(cursor);
        }
        let toast_layout = draw_status_toast(state, style, window)?;
        let toast_layout = if toast_layout.is_some() {
            toast_layout
        } else {
            draw_lsp_loading_toast(state, style, window)?
        };
        if force_hide_cursor {
            hide_cursor(window);
        } else if let Some(cursor) = cursor_spec {
            let cursor_hidden_by_perf = perf_popup_layout
                .is_some_and(|layout| popup_occludes_cursor(layout, cursor.x, cursor.y));
            let cursor_hidden_by_toast = toast_layout
                .is_some_and(|layout| popup_occludes_cursor(layout, cursor.x, cursor.y));
            if cursor_hidden_by_perf || cursor_hidden_by_toast {
                hide_cursor(window);
            } else {
                window.request_cursor(cursor);
            }
        }
        return Ok(());
    }

    state.ensure_rain_animation(text_w, text_h, editor_text, background_style);

    if let Some(animation) = state.active_rain_animation() {
        draw_relative_line_numbers(
            window,
            background_style,
            gutter_w,
            text_h,
            show_git_marker_column,
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
            animation.first_line(),
            state.active_git_diff(),
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
    let diagnostic_lines = state.active_diagnostic_lines(snapshot.first_line, snapshot.lines.len());
    let snippet_placeholders =
        state.active_snippet_placeholder_ranges(snapshot.first_line, snapshot.lines.len());
    perf.overlays += overlay_start.elapsed();

    draw_relative_line_numbers(
        window,
        background_style,
        gutter_w,
        text_h,
        show_git_marker_column,
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
        snapshot.first_line,
        state.active_git_diff(),
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
            let use_lexical_fallback = syntax_language.is_none()
                || syntax_language
                    .is_some_and(|language| view.syntax_highlighter.has_stale_cache_for(language));
            let syntax_spans = view
                .syntax_highlighter
                .visible_line_spans_for_display_cached(
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
                &snippet_placeholders,
                &diagnostic_lines,
                visual_selection,
                one_shot_highlight,
                use_lexical_fallback,
            )?;
            let lines_time = lines_start.elapsed();
            Ok::<_, minui::Error>((syntax_time, overlay_time, lines_time))
        })?;
    perf.syntax += syntax_time;
    perf.overlays += overlay_time;
    perf.lines += lines_time;
    state.advance_one_shot_highlight();

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
    } else if let Some(popup) = state.completion_popup() {
        if spec.visible {
            let cursor_x = spec.x.saturating_add(content_x);
            if let Some(preview) = state.completion_preview() {
                let preview_width =
                    clipped_cell_width(&preview.text, vw.saturating_sub(cursor_x) as usize);
                let inline_diagnostic = diagnostic_lines.get(&active_cursor_line);
                let source_line = inline_diagnostic.map(|_| {
                    state
                        .session
                        .active_buffer()
                        .line_string(active_cursor_line)
                });
                if let Some(source_line) = source_line.as_deref() {
                    clear_inline_diagnostic(
                        window,
                        spec.y,
                        content_x,
                        source_line,
                        scroll_x,
                        text_w as usize,
                        background_style,
                    )?;
                }
                draw_completion_preview(
                    window,
                    style,
                    cursor_x,
                    spec.y,
                    vw.saturating_sub(cursor_x),
                    &preview.text,
                    &preview.suffix,
                )?;
                if let Some((diagnostic, source_line)) =
                    inline_diagnostic.zip(source_line.as_deref())
                {
                    draw_inline_diagnostic_shifted(
                        window,
                        spec.y,
                        content_x,
                        source_line,
                        scroll_x,
                        text_w as usize,
                        background_style,
                        diagnostic,
                        preview_width,
                    )?;
                }
            }
            draw_completion_popup(&popup, style, window, cursor_x, spec.y, text_h)?;
            cursor_spec = Some(minui::window::CursorSpec {
                x: cursor_x,
                y: spec.y,
                visible: true,
            });
        }
    } else if spec.visible {
        let cursor_x = spec.x.saturating_add(content_x);
        if state.symbol_info_popup(vw).is_some() {
            state.clamp_symbol_info_scroll(vw);
            let popup = state
                .symbol_info_popup(vw)
                .expect("symbol info popup should still exist after clamping");
            draw_symbol_info_popup(&popup, style, window, cursor_x, spec.y)?;
            cursor_spec = Some(minui::window::CursorSpec {
                x: cursor_x,
                y: spec.y,
                visible: true,
            });
        } else {
            cursor_spec = Some(minui::window::CursorSpec {
                x: spec.x.saturating_add(content_x),
                y: spec.y,
                visible: true,
            });
        }
    }

    let toast_layout = draw_status_toast(state, style, window)?;
    if toast_layout.is_none() {
        let _ = draw_lsp_loading_toast(state, style, window)?;
    }

    if let Some(cursor) = cursor_spec {
        let cursor_hidden_by_perf = perf_popup_layout
            .is_some_and(|layout| popup_occludes_cursor(layout, cursor.x, cursor.y));
        let cursor_hidden_by_toast =
            toast_layout.is_some_and(|layout| popup_occludes_cursor(layout, cursor.x, cursor.y));
        if cursor_hidden_by_perf || cursor_hidden_by_toast {
            hide_cursor(window);
        } else {
            window.request_cursor(cursor);
        }
    }

    Ok(())
}

fn draw_lsp_loading_toast(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<Option<ui::widgets::popup::PopupLayout>> {
    let Some(message) = state.active_lsp_loading_toast(Instant::now()) else {
        return Ok(None);
    };
    let (term_w, term_h) = window.get_size();
    let width = (cell_width(&message, TabPolicy::Fixed(4)) as u16)
        .saturating_add(2)
        .min(term_w.saturating_sub(2));
    if width <= 2 || term_h <= 2 {
        return Ok(None);
    }
    let popup_w = width.saturating_add(2);
    let x = term_w.saturating_sub(popup_w.saturating_add(1));
    let layout = ui::widgets::popup::draw_popup_frame_at(
        window,
        x,
        0,
        width,
        1,
        "",
        ui::widgets::popup::PopupChrome::command_line(style),
    )?;
    let mut view = ui::widgets::popup::popup_window_view(window, layout);
    view.write_str_colored(0, 1, &message, style.command_line.text)?;
    Ok(Some(layout))
}

fn draw_gutter_padding(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    padding_w: u16,
    first_line: usize,
    git_diff: Option<&app::GitDiffSnapshot>,
) -> minui::Result<()> {
    if text_h == 0 {
        return Ok(());
    }

    let pad = " ".repeat(padding_w as usize);
    let color = ColorPair::new(style.theme.bg, style.theme.bg);
    for row in 0..text_h {
        if padding_w > 0 {
            window.write_str_colored(row, gutter_w, &pad, color)?;
        }
        let line_idx = first_line.saturating_add(row as usize);
        let Some(kind) = git_diff.and_then(|diff| diff.marker_for_line(line_idx)) else {
            continue;
        };
        let (glyph, colors) = style.git.gutter_marker(kind);
        window.write_str_colored(row, 0, glyph, colors)?;
    }
    Ok(())
}

fn git_marker_column_visible(git_diff: Option<&app::GitDiffSnapshot>) -> bool {
    git_diff.is_some_and(|diff| !diff.stats.is_empty())
}

fn line_number_gutter_width(total_lines: usize, show_git_marker_column: bool) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    let git_marker_width = u16::from(show_git_marker_column);
    digits.saturating_add(git_marker_width).saturating_add(1)
}

fn draw_relative_line_numbers(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    show_git_marker_column: bool,
    first_line: usize,
    cursor_line: usize,
    total_lines: usize,
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let sep_x = gutter_w.saturating_sub(1);
    let marker_offset = u16::from(show_git_marker_column);
    let number_w = gutter_w.saturating_sub(marker_offset).saturating_sub(1) as usize;
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
            window.write_str_colored(row, marker_offset, &text, color)?;
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
            draw_highlight_spaces(
                window,
                row,
                col,
                0,
                width_cells,
                normal_color.fg,
                highlight_layers,
            )?;
            if let Some((visible_col, bg)) = color_column
                && visible_col < width_cells
                && highlight_bg_at_cell(highlight_layers, visible_col).is_none()
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

    draw_highlight_spaces(
        window,
        row,
        col,
        used_cells,
        width_cells,
        normal_color.fg,
        highlight_layers,
    )?;

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
        && highlight_bg_at_cell(highlight_layers, visible_col).is_none()
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

fn draw_highlight_spaces(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    start_cell: usize,
    width_cells: usize,
    fg: Color,
    highlight_layers: &[(&[bool], Color)],
) -> minui::Result<()> {
    let mut cell = start_cell;
    while cell < width_cells {
        let Some(bg) = highlight_bg_at_cell(highlight_layers, cell) else {
            cell += 1;
            continue;
        };

        let run_start = cell;
        cell += 1;
        while cell < width_cells && highlight_bg_at_cell(highlight_layers, cell) == Some(bg) {
            cell += 1;
        }
        let spaces = " ".repeat(cell - run_start);
        window.write_str_colored(
            row,
            col.saturating_add(run_start as u16),
            &spaces,
            ColorPair::new(fg, bg),
        )?;
    }

    Ok(())
}

fn highlight_bg_at_cell(highlight_layers: &[(&[bool], Color)], cell: usize) -> Option<Color> {
    highlight_layers
        .iter()
        .find_map(|(cells, bg)| cells.get(cell).copied().unwrap_or(false).then_some(*bg))
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

fn draw_modal_popup_background(
    state: &mut EditorState,
    style: UiStyle,
    background_style: UiStyle,
    window: &mut dyn Window,
    width: u16,
    text_height: u16,
    editor_text: ColorPair,
    fallback_buffer_id: Option<BufferId>,
    inner_size: (u16, u16),
) -> minui::Result<()> {
    draw_popup_background(
        state,
        background_style,
        window,
        width,
        text_height,
        editor_text,
        fallback_buffer_id,
    )?;
    let (inner_w, inner_h) = inner_size;
    state.set_viewport_size(
        inner_w as usize,
        inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );
    build_editor_status_bar(state, style).draw(window)
}

fn finder_popup_inner_size(term_w: u16, term_h: u16, style: UiStyle) -> (u16, u16) {
    crate::ui::widgets::popup::popup_inner_size(
        term_w,
        term_h,
        style.finder.width_percent,
        style.finder.height_percent,
        style.finder.min_width,
        style.finder.min_height,
    )
}

fn draw_popup_background(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    width: u16,
    height: u16,
    editor_text: ColorPair,
    fallback_buffer_id: Option<BufferId>,
) -> minui::Result<()> {
    if state.panes().len() > 1 {
        let active_before_draw = state.session.active_id();
        state.sync_active_pane_view();
        draw_split_editor_panes(state, style, window, width, height, editor_text, false)?;
        let _ = state.session.activate(active_before_draw);
        return Ok(());
    }

    if let Some(buffer_id) = fallback_buffer_id {
        draw_buffer_snapshot_for_id(
            state,
            style,
            buffer_id,
            width,
            height,
            editor_text,
            window,
            None,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )?;
    }
    Ok(())
}

fn draw_split_editor_panes(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    width: u16,
    height: u16,
    editor_text: ColorPair,
    dim_inactive: bool,
) -> minui::Result<()> {
    let rects = state.pane_rects(width, height);
    for rect in rects.iter().copied() {
        let Some((buffer_id, view)) = state
            .panes()
            .iter()
            .find(|pane| pane.id == rect.pane_id)
            .map(|pane| (pane.buffer_id, pane.view.clone()))
        else {
            continue;
        };
        state
            .views
            .entry(buffer_id)
            .or_default()
            .copy_pane_state_from(&view);
        let pane_style = if !dim_inactive || rect.pane_id == state.active_pane_id() {
            style
        } else {
            style.dimmed()
        };
        let pane_text = if !dim_inactive || rect.pane_id == state.active_pane_id() {
            editor_text
        } else {
            ColorPair::new(pane_style.theme.white, pane_style.theme.bg)
        };
        let mut pane_window = WindowView {
            window,
            x_offset: rect.x,
            y_offset: rect.y,
            scroll_x: 0,
            scroll_y: 0,
            width: rect.width,
            height: rect.height,
        };
        let is_active_pane = rect.pane_id == state.active_pane_id();
        if is_active_pane && state.active_rain_animation().is_some() {
            draw_active_split_rain_pane(
                state,
                pane_style,
                rect.width,
                rect.height,
                &mut pane_window,
            )?;
        } else {
            let visual_selection = is_active_pane
                .then(|| state.active_visual_selection())
                .flatten();
            let one_shot_highlight = is_active_pane.then(|| state.one_shot_highlight()).flatten();
            let total_lines = state
                .session
                .buffer(buffer_id)
                .map_or(0, |buffer| buffer.len_lines().max(1));
            let (_, scroll_y) = view.cursor.viewport_scroll();
            let first_line = scroll_y.min(total_lines.saturating_sub(1));
            let visible_len = (rect.height as usize).min(total_lines.saturating_sub(first_line));
            let diagnostic_lines = if is_active_pane {
                state.active_diagnostic_lines(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            let snippet_placeholders = if is_active_pane {
                state.active_snippet_placeholder_ranges(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            let search_highlights = if is_active_pane {
                state.active_search_highlight_ranges(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            draw_buffer_snapshot_for_id(
                state,
                pane_style,
                buffer_id,
                rect.width,
                rect.height,
                pane_text,
                &mut pane_window,
                visual_selection,
                one_shot_highlight,
                &search_highlights,
                &diagnostic_lines,
                &snippet_placeholders,
            )?;
        }
        state.sync_rendered_pane_view(rect.pane_id, buffer_id);
    }
    state.restore_active_pane_view();
    draw_split_lines(window, style, &rects, width, height)
}

fn draw_split_lines(
    window: &mut dyn Window,
    style: UiStyle,
    rects: &[app::PaneRect],
    width: u16,
    height: u16,
) -> minui::Result<()> {
    let line_color = ColorPair::new(style.theme.light_gray, style.theme.bg);
    let mut line_cells = vec![false; width as usize * height as usize];
    for rect in rects {
        if rect.x > 0 {
            let x = rect.x - 1;
            if x < width {
                let start_y = rect.y.saturating_sub(1);
                for y in start_y..rect.y.saturating_add(rect.height).min(height) {
                    line_cells[y as usize * width as usize + x as usize] = true;
                }
            }
        }
        if rect.y > 0 {
            let y = rect.y - 1;
            if y < height {
                let start_x = rect.x.saturating_sub(1);
                for x in start_x..rect.x.saturating_add(rect.width).min(width) {
                    line_cells[y as usize * width as usize + x as usize] = true;
                }
            }
        }
    }

    for y in 0..height {
        for x in 0..width {
            let idx = y as usize * width as usize + x as usize;
            if !line_cells[idx] {
                continue;
            }
            let up = y > 0 && line_cells[(y - 1) as usize * width as usize + x as usize];
            let down = y + 1 < height && line_cells[(y + 1) as usize * width as usize + x as usize];
            let left = x > 0 && line_cells[y as usize * width as usize + (x - 1) as usize];
            let right = x + 1 < width && line_cells[y as usize * width as usize + (x + 1) as usize];
            let glyph = split_line_glyph(up, down, left, right);
            window.write_str_colored(y, x, glyph, line_color)?;
        }
    }
    Ok(())
}

fn split_line_glyph(up: bool, down: bool, left: bool, right: bool) -> &'static str {
    match (up, down, left, right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (true, false, true, true) => "┴",
        (false, true, true, true) => "┬",
        (true, false, false, true) => "└",
        (true, false, true, false) => "┘",
        (false, true, false, true) => "┌",
        (false, true, true, false) => "┐",
        (true, true, _, _) => "│",
        (_, _, true, true) => "─",
        (true, false, false, false) | (false, true, false, false) => "│",
        (false, false, true, false) | (false, false, false, true) => "─",
        _ => " ",
    }
}

fn active_split_cursor(
    state: &mut EditorState,
    width: u16,
    height: u16,
) -> Option<minui::window::CursorSpec> {
    let rect = state
        .pane_rects(width, height)
        .into_iter()
        .find(|rect| rect.pane_id == state.active_pane_id())?;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let show_git_marker_column = git_marker_column_visible(state.active_git_diff());
    let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let text_w = rect.width.saturating_sub(content_x);
    let spec = state.with_active_buffer_view_mut(|buffer, view| {
        view.cursor
            .cursor_spec(buffer, text_w as usize, rect.height as usize)
    });
    spec.visible.then_some(minui::window::CursorSpec {
        x: rect.x.saturating_add(content_x).saturating_add(spec.x),
        y: rect.y.saturating_add(spec.y),
        visible: true,
    })
}

struct SplitCursorContext {
    x: u16,
    y: u16,
    content_x: u16,
    text_w: u16,
    height: u16,
    scroll_x: usize,
}

fn active_split_cursor_context(
    state: &mut EditorState,
    width: u16,
    height: u16,
) -> Option<SplitCursorContext> {
    let rect = state
        .pane_rects(width, height)
        .into_iter()
        .find(|rect| rect.pane_id == state.active_pane_id())?;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let show_git_marker_column = git_marker_column_visible(state.active_git_diff());
    let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
    let local_content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let content_x = rect.x.saturating_add(local_content_x);
    let text_w = rect.width.saturating_sub(local_content_x);
    let (spec, scroll_x) = state.with_active_buffer_view_mut(|buffer, view| {
        let (scroll_x, _) = view.cursor.viewport_scroll();
        (
            view.cursor
                .cursor_spec(buffer, text_w as usize, rect.height as usize),
            scroll_x,
        )
    });
    spec.visible.then_some(SplitCursorContext {
        x: content_x.saturating_add(spec.x),
        y: rect.y.saturating_add(spec.y),
        content_x,
        text_w,
        height: rect.height,
        scroll_x,
    })
}

fn draw_active_split_rain_pane(
    state: &mut EditorState,
    style: UiStyle,
    width: u16,
    height: u16,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let show_git_marker_column = git_marker_column_visible(state.active_git_diff());
    let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let active_cursor_line = state.active_cursor_pos().line;
    let Some(animation) = state.active_rain_animation() else {
        return Ok(());
    };

    draw_relative_line_numbers(
        window,
        style,
        gutter_w,
        height,
        show_git_marker_column,
        animation.first_line(),
        active_cursor_line,
        total_lines,
    )?;
    draw_gutter_padding(
        window,
        style,
        gutter_w,
        height,
        GUTTER_CONTENT_PADDING,
        animation.first_line(),
        state.active_git_diff(),
    )?;
    animation.draw(
        window,
        0,
        content_x,
        width.saturating_sub(content_x) as usize,
        height as usize,
    )
}

fn draw_buffer_snapshot_for_id(
    state: &mut EditorState,
    style: UiStyle,
    buffer_id: BufferId,
    width: u16,
    height: u16,
    colors: ColorPair,
    window: &mut dyn Window,
    visual_selection: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    one_shot_highlight: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    search_highlights: &BTreeMap<usize, Vec<std::ops::Range<usize>>>,
    diagnostic_lines: &BTreeMap<usize, app::DiagnosticLine>,
    snippet_placeholders: &BTreeMap<usize, Vec<std::ops::Range<usize>>>,
) -> minui::Result<()> {
    state.refresh_git_diff_for_buffer(buffer_id);
    let git_diff = state.git_diff_for_buffer(buffer_id).cloned();
    let show_git_marker_column = git_marker_column_visible(git_diff.as_ref());
    let syntax_language = state
        .session
        .meta(buffer_id)
        .and_then(|meta| language_for_path(meta.path.as_deref()));
    let Some(result) = state.with_buffer_view_mut(buffer_id, |buffer, view| {
        let cursor = view.cursor.cursor;
        let total_lines = buffer.len_lines().max(1);
        let gutter_w = line_number_gutter_width(total_lines, show_git_marker_column);
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
        let use_lexical_fallback = syntax_language.is_none()
            || syntax_language
                .is_some_and(|language| view.syntax_highlighter.has_stale_cache_for(language));
        let syntax_spans = view
            .syntax_highlighter
            .visible_line_spans_for_display_cached(
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
            show_git_marker_column,
            snapshot.first_line,
            view.cursor.cursor.line,
            total_lines,
        )?;
        draw_gutter_padding(
            window,
            style,
            gutter_w,
            height,
            GUTTER_CONTENT_PADDING,
            snapshot.first_line,
            git_diff.as_ref(),
        )?;

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
            search_highlights,
            snippet_placeholders,
            diagnostic_lines,
            visual_selection,
            one_shot_highlight,
            use_lexical_fallback,
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
    snippet_placeholders: &BTreeMap<usize, Vec<std::ops::Range<usize>>>,
    diagnostic_lines: &BTreeMap<usize, app::DiagnosticLine>,
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
        let syntax_line_spans = syntax_spans.and_then(|rows| rows.get(row));
        let merged_line_spans = match (syntax_line_spans, fallback_line_spans.as_deref()) {
            (Some(syntax), Some(fallback)) => Some(merge_line_spans_for_display(syntax, fallback)),
            _ => None,
        };
        let syntax_line_spans = merged_line_spans
            .as_deref()
            .or(syntax_line_spans)
            .or(fallback_line_spans.as_deref());
        let highlighted_chars = delimiter_highlights
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let visible_indent_guides = active_scope_guides
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let diagnostic_line = diagnostic_lines.get(&line_idx);
        let snippet_ranges = snippet_placeholders
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let has_search_highlights = search_highlights
            .get(&line_idx)
            .is_some_and(|ranges| !ranges.is_empty());
        let diagnostic_cells = diagnostic_line.map(|diagnostic| {
            selected_visible_cells(
                &source_line,
                scroll_x,
                text_w,
                diagnostic.start_col,
                diagnostic
                    .end_col
                    .max(diagnostic.start_col.saturating_add(1)),
            )
        });
        let transient_selection = visual_selection
            .map(|(selection, mode)| (selection, mode, style.theme.selection_bg))
            .or_else(|| {
                one_shot_highlight
                    .map(|(selection, mode)| (selection, mode, style.theme.light_purple))
            });

        if transient_selection.is_none()
            && !has_search_highlights
            && syntax_line_spans.is_none_or(|spans| spans.is_empty())
            && diagnostic_line.is_none()
            && snippet_ranges.is_empty()
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
            if let Some(diagnostic) = diagnostic_line {
                draw_inline_diagnostic(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    style,
                    diagnostic,
                )?;
            }
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
            if let Some(selected_cells) = visual_selection_visible_cells(
                buffer,
                &source_line,
                selection,
                mode,
                line_idx,
                scroll_x,
                text_w,
            ) {
                let highlight_empty_line = source_line.is_empty();
                let highlight_layers = if let Some(search_cells) = search_cells.as_ref() {
                    let mut layers = vec![
                        (selected_cells.as_slice(), selection_bg),
                        (search_cells.as_slice(), style.theme.selection_bg),
                    ];
                    if let Some(diagnostic) = diagnostic_cells.as_ref().zip(diagnostic_line) {
                        layers.push((
                            diagnostic.0.as_slice(),
                            style.diagnostic_inline.background(diagnostic.1.severity),
                        ));
                    }
                    layers
                } else {
                    let mut layers = vec![(selected_cells.as_slice(), selection_bg)];
                    if let Some(diagnostic) = diagnostic_cells.as_ref().zip(diagnostic_line) {
                        layers.push((
                            diagnostic.0.as_slice(),
                            style.diagnostic_inline.background(diagnostic.1.severity),
                        ));
                    }
                    layers
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
                    default_colors,
                    style,
                    syntax_line_spans,
                )?;
                draw_snippet_placeholders(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    snippet_ranges,
                    style,
                )?;
                if let Some(diagnostic) = diagnostic_line {
                    draw_inline_diagnostic(
                        window,
                        row as u16,
                        content_x,
                        &source_line,
                        scroll_x,
                        text_w,
                        style,
                        diagnostic,
                    )?;
                }
                continue;
            }
        }

        if let Some(search_cells) = search_cells.as_ref()
            && search_cells.iter().any(|selected| *selected)
        {
            let highlight_layers =
                if let Some(diagnostic) = diagnostic_cells.as_ref().zip(diagnostic_line) {
                    vec![
                        (search_cells.as_slice(), style.theme.selection_bg),
                        (
                            diagnostic.0.as_slice(),
                            style.diagnostic_inline.background(diagnostic.1.severity),
                        ),
                    ]
                } else {
                    vec![(search_cells.as_slice(), style.theme.selection_bg)]
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
                default_colors,
                style,
                syntax_line_spans,
            )?;
            draw_snippet_placeholders(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                snippet_ranges,
                style,
            )?;
            if let Some(diagnostic) = diagnostic_line {
                draw_inline_diagnostic(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    style,
                    diagnostic,
                )?;
            }
            continue;
        }

        if let Some(spans) = syntax_line_spans
            && !spans.is_empty()
        {
            if let Some(diagnostic) = diagnostic_cells.as_ref().zip(diagnostic_line) {
                let highlight_layers = [(
                    diagnostic.0.as_slice(),
                    style.diagnostic_inline.background(diagnostic.1.severity),
                )];
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
                    Some(spans),
                    &highlight_layers,
                    false,
                )?;
            } else {
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
            }
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
                default_colors,
                style,
                Some(spans),
            )?;
            draw_snippet_placeholders(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                snippet_ranges,
                style,
            )?;
            if let Some(diagnostic) = diagnostic_line {
                draw_inline_diagnostic(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    style,
                    diagnostic,
                )?;
            }
            continue;
        }

        if let Some(diagnostic) = diagnostic_cells.as_ref().zip(diagnostic_line) {
            let highlight_layers = [(
                diagnostic.0.as_slice(),
                style.diagnostic_inline.background(diagnostic.1.severity),
            )];
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
                None,
                &highlight_layers,
                false,
            )?;
        } else {
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
        }
        draw_snippet_placeholders(
            window,
            row as u16,
            content_x,
            &source_line,
            scroll_x,
            text_w,
            snippet_ranges,
            style,
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
            default_colors,
            style,
            syntax_line_spans,
        )?;
        if let Some(diagnostic) = diagnostic_line {
            draw_inline_diagnostic(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                style,
                diagnostic,
            )?;
        }
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

fn draw_snippet_placeholders(
    window: &mut dyn Window,
    row: u16,
    content_x: u16,
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    ranges: &[std::ops::Range<usize>],
    style: UiStyle,
) -> minui::Result<()> {
    if ranges.is_empty() || text_w == 0 {
        return Ok(());
    }

    let visible_end = scroll_x.saturating_add(text_w);
    let color = ColorPair::new(style.theme.dark_gray, style.theme.bg);
    let mut cell = 0usize;
    let mut char_col = 0usize;
    for grapheme in source_line.graphemes(true) {
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
        if grapheme == "\t" {
            let spaces = " ".repeat(width.max(1));
            window.write_str_colored(
                row,
                content_x.saturating_add(visible_x as u16),
                &spaces,
                color,
            )?;
        } else {
            window.write_str_colored(
                row,
                content_x.saturating_add(visible_x as u16),
                grapheme,
                color,
            )?;
        }
    }
    Ok(())
}

fn draw_inline_diagnostic(
    window: &mut dyn Window,
    row: u16,
    content_x: u16,
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    style: UiStyle,
    diagnostic: &app::DiagnosticLine,
) -> minui::Result<()> {
    draw_inline_diagnostic_shifted(
        window,
        row,
        content_x,
        source_line,
        scroll_x,
        text_w,
        style,
        diagnostic,
        0,
    )
}

fn clear_inline_diagnostic(
    window: &mut dyn Window,
    row: u16,
    content_x: u16,
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    style: UiStyle,
) -> minui::Result<()> {
    if text_w == 0 {
        return Ok(());
    }

    let Some(start_cell) = inline_diagnostic_start_cell(source_line, scroll_x, text_w, 0) else {
        return Ok(());
    };
    let blank = " ".repeat(text_w.saturating_sub(start_cell));
    window.write_str_colored(
        row,
        content_x.saturating_add(start_cell as u16),
        &blank,
        ColorPair::new(style.theme.white, style.theme.bg),
    )
}

fn draw_inline_diagnostic_shifted(
    window: &mut dyn Window,
    row: u16,
    content_x: u16,
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    style: UiStyle,
    diagnostic: &app::DiagnosticLine,
    shift_cells: usize,
) -> minui::Result<()> {
    if text_w == 0 {
        return Ok(());
    }

    let Some(start_cell) = inline_diagnostic_start_cell(source_line, scroll_x, text_w, shift_cells)
    else {
        return Ok(());
    };
    let available = text_w.saturating_sub(start_cell);
    let inline_text = ui::widgets::popup::clip_text_to_cells(&diagnostic.inline_text, available);
    if inline_text.is_empty() {
        return Ok(());
    }

    let colors = style.diagnostic_inline.colors(diagnostic.severity);

    window.write_str_colored(
        row,
        content_x.saturating_add(start_cell as u16),
        &inline_text,
        colors,
    )
}

fn inline_diagnostic_start_cell(
    source_line: &str,
    scroll_x: usize,
    text_w: usize,
    shift_cells: usize,
) -> Option<usize> {
    let line_width = visible_content_cell_width(source_line, scroll_x, text_w);
    let start_cell = line_width.saturating_add(shift_cells).saturating_add(5);
    (start_cell < text_w).then_some(start_cell)
}

fn visible_content_cell_width(source_line: &str, scroll_x: usize, max_cells: usize) -> usize {
    if max_cells == 0 || source_line.is_empty() {
        return 0;
    }

    let mut consumed_cells = 0usize;
    let mut visible_cells = 0usize;

    for grapheme in source_line.graphemes(true) {
        let grapheme_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        if consumed_cells.saturating_add(grapheme_width) <= scroll_x {
            consumed_cells = consumed_cells.saturating_add(grapheme_width);
            continue;
        }

        let remaining_cells = max_cells.saturating_sub(visible_cells);
        if remaining_cells == 0 {
            break;
        }

        visible_cells = visible_cells.saturating_add(grapheme_width.min(remaining_cells));
        if visible_cells >= max_cells {
            break;
        }
    }

    visible_cells
}

fn clipped_cell_width(text: &str, max_cells: usize) -> usize {
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = cell_width(&ch.to_string(), TabPolicy::Fixed(4)) as usize;
        if width.saturating_add(ch_width) > max_cells {
            break;
        }
        width = width.saturating_add(ch_width);
    }
    width
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

fn visual_selection_visible_cells(
    buffer: &redox_core::TextBuffer,
    source_line: &str,
    selection: redox_core::Selection,
    mode: redox_core::VisualModeKind,
    line_idx: usize,
    scroll_x: usize,
    width_cells: usize,
) -> Option<Vec<bool>> {
    let cells = match mode {
        redox_core::VisualModeKind::Line => {
            let (start_line, end_line) = selection.line_range();
            if line_idx < start_line || line_idx > end_line {
                return None;
            }
            vec![true; width_cells]
        }
        redox_core::VisualModeKind::Block => {
            let (start, end) = selection.ordered();
            if line_idx < start.line || line_idx > end.line {
                return None;
            }
            let left = start.col.min(end.col);
            let right = start.col.max(end.col).saturating_add(1);
            visible_cell_range(left, right, scroll_x, width_cells)
        }
        redox_core::VisualModeKind::Char => {
            let (start, end) = selection.ordered();
            if line_idx < start.line || line_idx > end.line {
                return None;
            }

            if start.line < line_idx && line_idx < end.line {
                vec![true; width_cells]
            } else if let Some(range) =
                buffer.visual_selection_char_range_on_line(selection, mode, line_idx)
            {
                let mut cells = selected_visible_cells(
                    source_line,
                    scroll_x,
                    width_cells,
                    range.start,
                    range.end,
                );
                if start.line != end.line && line_idx == start.line {
                    let line_width = line_cell_width(source_line);
                    mark_visible_cell_range(&mut cells, line_width, usize::MAX, scroll_x);
                }
                cells
            } else if start.line < end.line && source_line.is_empty() {
                vec![true; width_cells]
            } else {
                return None;
            }
        }
    };

    cells.iter().any(|cell| *cell).then_some(cells)
}

fn line_cell_width(source_line: &str) -> usize {
    source_line
        .graphemes(true)
        .map(|g| (cell_width(g, TabPolicy::Fixed(4)) as usize).max(1))
        .sum()
}

fn visible_cell_range(
    start_cell: usize,
    end_cell: usize,
    scroll_x: usize,
    width_cells: usize,
) -> Vec<bool> {
    let mut cells = vec![false; width_cells];
    mark_visible_cell_range(&mut cells, start_cell, end_cell, scroll_x);
    cells
}

fn mark_visible_cell_range(
    cells: &mut [bool],
    start_cell: usize,
    end_cell: usize,
    scroll_x: usize,
) {
    if cells.is_empty() || start_cell >= end_cell {
        return;
    }

    let visible_start = start_cell.saturating_sub(scroll_x);
    let visible_end = end_cell.saturating_sub(scroll_x).min(cells.len());
    if visible_start >= visible_end {
        return;
    }

    for cell in &mut cells[visible_start..visible_end] {
        *cell = true;
    }
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
    fn visual_selection_visible_cells_connect_blank_lines() {
        let buffer = redox_core::TextBuffer::from_str("alpha\n\nomega\n");
        let selection =
            redox_core::Selection::new(redox_core::Pos::new(0, 1), redox_core::Pos::new(2, 2));

        for mode in [
            redox_core::VisualModeKind::Char,
            redox_core::VisualModeKind::Line,
            redox_core::VisualModeKind::Block,
        ] {
            let cells = visual_selection_visible_cells(&buffer, "", selection, mode, 1, 0, 8)
                .expect("blank line should be visually selected");
            assert!(cells.iter().any(|selected| *selected));
        }
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
    let symbol_info_keybind = KeybindAction::Custom("trigger-symbol-info".to_string());
    if window
        .keyboard_mut()
        .add_keybind("ctrl-i", symbol_info_keybind.clone())
        .is_err()
    {
        let _ = window
            .keyboard_mut()
            .add_keybind("ctrl+i", symbol_info_keybind);
    }
    let completion_keybind = KeybindAction::Custom("trigger-completion".to_string());
    if window
        .keyboard_mut()
        .add_keybind("ctrl-shift-k", completion_keybind.clone())
        .is_err()
    {
        let _ = window
            .keyboard_mut()
            .add_keybind("ctrl+shift+k", completion_keybind);
    }
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
        state.poll_lsp();
        state.poll_finder_results();
        let now = Instant::now();
        state.poll_external_file_changes(now);
        state.expire_status_message(now);

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
