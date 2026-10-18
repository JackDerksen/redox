//! Drawing a buffer snapshot with its syntax, search, selection and diagnostic overlays.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use minui::prelude::render::{Color, Window};
use redox_core::{BufferId, TextBuffer};

use crate::GUTTER_CONTENT_PADDING;
use crate::app::state::{HighlightKind, OneShotHighlight};
use crate::app::{EditorState, FramePerfSample, UndoTreeSurfaceRole};
use crate::ui::overlays::{
    HighlightFade, LineDecorations, YankHighlight, active_delimiter_highlights,
    active_scope_indent_guides, draw_delimiter_highlights, draw_indent_guides,
};
use crate::ui::render::LineViewport;
use crate::ui::syntax::{
    VisibleLineSyntaxSpans, draw_line_with_syntax, lexical_fallback_line_spans,
    scope_guides_enabled,
};
use crate::ui::text_style::TextStyle;
use crate::ui::{
    TextViewport, UNDO_TREE_HEADER_ROWS, UiStyle, draw_undo_tree_lines,
    draw_undo_tree_preview_lines, language_for_path,
};
use crate::view::cells::{
    occupied_visible_cells, search_highlight_cells, selected_visible_cells,
    visual_selection_visible_cells,
};
use crate::view::gutter::{
    PaneGutterLayout, draw_active_gutter, draw_gutter_padding, draw_line_numbers,
    line_number_gutter_width, marker_column_visible,
};
use crate::view::line::{
    LineHighlights, draw_inline_diagnostic, draw_line_with_highlights, draw_plain_line,
    draw_snippet_placeholders, draw_visible_ascii_plain_line, visible_color_column,
};
use crate::view::panes::CursorContext;

pub(crate) struct BufferDrawOptions {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) has_line_numbers: bool,
    pub(crate) colors: TextStyle,
}

pub(crate) struct BufferHighlights<'a> {
    pub(crate) visual_selection: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    pub(crate) one_shot_highlight: Option<OneShotHighlight>,
    pub(crate) search_highlights: &'a BTreeMap<usize, crate::app::state::SearchLineHighlights>,
    pub(crate) diagnostic_lines: &'a BTreeMap<usize, crate::app::DiagnosticLine>,
    pub(crate) snippet_placeholders: &'a BTreeMap<usize, Vec<std::ops::Range<usize>>>,
}

pub(crate) struct SnapshotViewport {
    pub(crate) content_x: u16,
    pub(crate) scroll_x: usize,
    pub(crate) text_w: usize,
}

pub(crate) struct SnapshotStyle<'a> {
    pub(crate) default_colors: TextStyle,
    pub(crate) style: &'a UiStyle,
    pub(crate) syntax_spans: Option<VisibleLineSyntaxSpans<'a>>,
    pub(crate) lexical_fallback_enabled: bool,
}

pub(crate) struct SnapshotOverlays<'a> {
    pub(crate) delimiter_highlights: &'a BTreeMap<usize, Vec<usize>>,
    pub(crate) active_scope_guides: &'a BTreeMap<usize, Vec<usize>>,
    pub(crate) search_highlights: &'a BTreeMap<usize, crate::app::state::SearchLineHighlights>,
    pub(crate) substitution_colors: Option<TextStyle>,
    pub(crate) snippet_placeholders: &'a BTreeMap<usize, Vec<std::ops::Range<usize>>>,
    pub(crate) diagnostic_lines: &'a BTreeMap<usize, crate::app::DiagnosticLine>,
    pub(crate) visual_selection: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    pub(crate) one_shot_highlight: Option<OneShotHighlight>,
    pub(crate) focused_lines: Option<std::ops::Range<usize>>,
}

/// Draws the active buffer's gutter and text when it fills the editor area, returning where its
/// cursor sits when visible.
pub(crate) fn draw_active_buffer(
    state: &mut EditorState,
    background_style: &UiStyle,
    window: &mut dyn Window,
    gutter: &PaneGutterLayout,
    (text_w, text_h): (u16, u16),
    active_cursor_line: usize,
    perf: &mut FramePerfSample,
) -> minui::Result<Option<CursorContext>> {
    let content_x = gutter.content_x;
    let editor_text = background_style.editor_text;
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
            let snapshot = view.render_line_cache.snapshot(buffer, &viewport);
            let spec = view
                .cursor
                .cursor_spec(buffer, text_w as usize, text_h as usize);
            let snapshot_time = snapshot_start.elapsed();
            (snapshot, spec, scroll_x, snapshot_time)
        });
    perf.snapshot += snapshot_time;
    let overlay_start = Instant::now();
    let search_highlights =
        state.active_search_highlight_ranges(snapshot.first_line(), snapshot.line_count());
    let diagnostic_lines =
        state.active_diagnostic_lines(snapshot.first_line(), snapshot.line_count());
    let snippet_placeholders =
        state.active_snippet_placeholder_ranges(snapshot.first_line(), snapshot.line_count());
    perf.overlays += overlay_start.elapsed();

    draw_active_gutter(
        state,
        background_style,
        window,
        gutter,
        (text_h, snapshot.first_line(), active_cursor_line),
        visual_selection
            .map(|(selection, _)| selection.line_range())
            .or_else(|| one_shot_highlight.and_then(OneShotHighlight::yank_lines)),
    )?;

    let focus_scope = state.zen.enabled && state.zen.focus_scope;
    let substitution_colors = state
        .substitute_preview()
        .map(|preview| background_style.substitute_colors(preview.replacing));
    let (syntax_time, overlay_time, lines_time) =
        state.with_active_buffer_view_mut(|buffer, view| {
            let cursor = view.cursor.cursor;
            let syntax_start = Instant::now();
            let analysis_version = view.analysis_version();
            let scope_guides_enabled = scope_guides_enabled(syntax_language);
            let syntax_scope = (focus_scope || scope_guides_enabled)
                .then(|| {
                    view.syntax_highlighter.active_scope_for_display_cached(
                        buffer,
                        syntax_language,
                        analysis_version,
                        cursor,
                    )
                })
                .flatten();
            let focused_lines = focus_scope.then(|| {
                if let Some((selection, _)) = visual_selection {
                    view.syntax_highlighter
                        .visual_scope_lines_for_display_cached(
                            buffer,
                            syntax_language,
                            analysis_version,
                            selection,
                        )
                } else {
                    syntax_scope
                        .map(|scope| scope.lines())
                        .unwrap_or(cursor.line..cursor.line.saturating_add(1))
                }
            });
            let use_lexical_fallback = should_use_lexical_fallback(syntax_language);
            let syntax_spans = view
                .syntax_highlighter
                .visible_line_spans_for_display_cached(
                    syntax_language,
                    snapshot.first_line(),
                    snapshot.line_count(),
                );
            let syntax_time = syntax_start.elapsed();
            let overlay_start = Instant::now();
            let delimiter_analysis = view.delimiter_pair_cache.get_for_display();
            let delimiter_highlights = delimiter_analysis
                .map(|analysis| {
                    active_delimiter_highlights(
                        buffer,
                        syntax_scope,
                        cursor,
                        snapshot.first_line(),
                        snapshot.line_count(),
                        analysis,
                    )
                })
                .unwrap_or_default();
            let active_scope_guides = if scope_guides_enabled {
                active_scope_indent_guides(
                    syntax_scope,
                    buffer,
                    cursor,
                    (snapshot.first_line(), snapshot.line_count()),
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
                SnapshotViewport {
                    content_x,
                    scroll_x,
                    text_w: text_w as usize,
                },
                SnapshotStyle {
                    default_colors: editor_text,
                    style: background_style,
                    syntax_spans,
                    lexical_fallback_enabled: use_lexical_fallback,
                },
                SnapshotOverlays {
                    delimiter_highlights: &delimiter_highlights,
                    active_scope_guides: &active_scope_guides,
                    search_highlights: &search_highlights,
                    substitution_colors,
                    snippet_placeholders: &snippet_placeholders,
                    diagnostic_lines: &diagnostic_lines,
                    visual_selection,
                    one_shot_highlight,
                    focused_lines,
                },
            )?;
            let lines_time = lines_start.elapsed();
            Ok::<_, minui::Error>((syntax_time, overlay_time, lines_time))
        })?;
    perf.syntax += syntax_time;
    perf.overlays += overlay_time;
    perf.lines += lines_time;

    Ok(spec.visible.then_some(CursorContext {
        x: spec.x.saturating_add(content_x),
        y: spec.y,
        content_x,
        text_w,
        height: text_h,
        scroll_x,
    }))
}

pub(crate) fn draw_buffer_snapshot_for_id(
    state: &mut EditorState,
    style: &UiStyle,
    buffer_id: BufferId,
    options: BufferDrawOptions,
    window: &mut dyn Window,
    highlights: BufferHighlights<'_>,
) -> minui::Result<()> {
    let BufferDrawOptions {
        width,
        height,
        has_line_numbers,
        colors,
    } = options;
    let BufferHighlights {
        visual_selection,
        one_shot_highlight,
        search_highlights,
        diagnostic_lines,
        snippet_placeholders,
    } = highlights;

    let has_line_numbers = has_line_numbers && !(state.zen.enabled && state.zen.hide_gutter);
    let focus_scope = state.zen.enabled && state.zen.focus_scope;
    let substitution_colors = state
        .substitute_preview()
        .filter(|_| buffer_id == state.session.active_id())
        .map(|preview| style.substitute_colors(preview.replacing));
    let undo_tree_role = state.undo_tree_surface_role(buffer_id);
    let undo_tree_line_spans = state
        .undo_tree_line_spans(buffer_id)
        .cloned()
        .unwrap_or_else(|| Arc::new([]));
    let undo_tree_preview_separator_row = state.undo_tree_preview_separator_row(buffer_id);
    let git_diff = undo_tree_role
        .is_none()
        .then(|| state.git_diff_for_buffer(buffer_id).cloned())
        .flatten();
    let show_marker_column = has_line_numbers
        && state
            .session
            .buffer(buffer_id)
            .is_some_and(|buffer| marker_column_visible(buffer, git_diff.as_ref()));
    let syntax_language = undo_tree_role
        .is_none()
        .then(|| {
            state
                .session
                .meta(buffer_id)
                .and_then(|meta| language_for_path(meta.path.as_deref()))
        })
        .flatten();
    let source = state.session.buffer(buffer_id).cloned();
    let Some(result) = state.with_buffer_display_view_mut(buffer_id, |buffer, view, preview| {
        let (scroll_x, mut scroll_y) = view.cursor.viewport_scroll();
        let mut cursor = view.cursor.cursor;
        if let Some(preview) = preview
            && let Some(source) = &source
        {
            cursor = preview.display_position(source, cursor);
            scroll_y = preview
                .display_position(source, redox_core::Pos::new(scroll_y, 0))
                .line;
        }
        if let Some(role) = undo_tree_role {
            return match role {
                UndoTreeSurfaceRole::Tree => {
                    let content_height = height.saturating_sub(UNDO_TREE_HEADER_ROWS);
                    let (first_line, lines) =
                        visible_buffer_lines(buffer, scroll_y, content_height as usize);
                    draw_undo_tree_lines(
                        window,
                        width,
                        style.undo_tree,
                        &lines,
                        &undo_tree_line_spans,
                        (first_line, view.cursor.cursor.line),
                        style.icons_enabled,
                    )
                }
                UndoTreeSurfaceRole::Preview => {
                    let (first_line, lines) =
                        visible_buffer_lines(buffer, scroll_y, height as usize);
                    draw_undo_tree_preview_lines(
                        window,
                        width,
                        scroll_x,
                        style.undo_tree,
                        &lines,
                        (first_line, undo_tree_preview_separator_row),
                    )
                }
            };
        }

        let total_lines = buffer.len_lines().max(1);
        let gutter_w = if has_line_numbers {
            line_number_gutter_width(total_lines, u16::from(show_marker_column))
        } else {
            0
        };
        let content_x = if has_line_numbers {
            gutter_w.saturating_add(GUTTER_CONTENT_PADDING)
        } else {
            0
        };
        let text_w = width.saturating_sub(content_x);
        let viewport = TextViewport {
            scroll_x,
            scroll_y,
            width: text_w,
            height,
        };
        let snapshot = view.render_line_cache.snapshot(buffer, &viewport);
        let analysis_version = view.analysis_version();
        let scope_guides_enabled = scope_guides_enabled(syntax_language);
        let syntax_scope = (focus_scope || scope_guides_enabled)
            .then(|| {
                let source = source.as_ref().unwrap_or(buffer);
                view.syntax_highlighter
                    .active_scope_for_display_cached(
                        source,
                        syntax_language,
                        analysis_version,
                        view.cursor.cursor,
                    )
                    .map(|mut scope| {
                        if let Some(preview) = preview {
                            for pair in [&mut scope.extent, &mut scope.body] {
                                pair.start = preview.display_position(source, pair.start);
                                pair.end = preview.display_position(source, pair.end);
                            }
                        }
                        scope
                    })
            })
            .flatten();
        let focused_lines = focus_scope.then(|| {
            if let Some((selection, _)) = visual_selection.filter(|_| preview.is_none()) {
                view.syntax_highlighter
                    .visual_scope_lines_for_display_cached(
                        buffer,
                        syntax_language,
                        analysis_version,
                        selection,
                    )
            } else {
                syntax_scope
                    .map(|scope| scope.lines())
                    .unwrap_or(cursor.line..cursor.line.saturating_add(1))
            }
        });
        let use_lexical_fallback = should_use_lexical_fallback(syntax_language);
        let syntax_highlighter =
            preview.map_or(&view.syntax_highlighter, |preview| &preview.syntax);
        let syntax_spans = syntax_highlighter.visible_line_spans_for_display_cached(
            syntax_language,
            snapshot.first_line(),
            snapshot.line_count(),
        );
        let delimiter_analysis = preview
            .is_none()
            .then(|| view.delimiter_pair_cache.get_for_display())
            .flatten();
        let delimiter_highlights = delimiter_analysis
            .map(|analysis| {
                active_delimiter_highlights(
                    buffer,
                    syntax_scope,
                    cursor,
                    snapshot.first_line(),
                    snapshot.line_count(),
                    analysis,
                )
            })
            .unwrap_or_default();
        let active_scope_guides = if scope_guides_enabled {
            active_scope_indent_guides(
                syntax_scope,
                buffer,
                cursor,
                (snapshot.first_line(), snapshot.line_count()),
                scroll_x,
                width.saturating_sub(content_x) as usize,
                delimiter_analysis,
            )
        } else {
            BTreeMap::new()
        };

        if has_line_numbers {
            draw_line_numbers(
                window,
                style,
                (gutter_w, height),
                u16::from(show_marker_column),
                snapshot.first_line()..total_lines,
                cursor.line,
                visual_selection
                    .map(|(selection, _)| selection.line_range())
                    .or_else(|| one_shot_highlight.and_then(OneShotHighlight::yank_lines))
                    .filter(|_| preview.is_none()),
            )?;
            draw_gutter_padding(
                window,
                style,
                gutter_w,
                height,
                GUTTER_CONTENT_PADDING,
                snapshot.first_line(),
                (
                    git_diff.as_ref().filter(|_| preview.is_none()),
                    preview.is_none().then_some(buffer),
                ),
            )?;
        }

        let preview_highlights = preview.map(|preview| {
            preview.highlight_ranges(buffer, snapshot.first_line(), snapshot.line_count())
        });
        let empty_diagnostics = BTreeMap::new();
        let empty_snippets = BTreeMap::new();
        draw_snapshot_lines(
            window,
            buffer,
            &snapshot,
            SnapshotViewport {
                content_x,
                scroll_x,
                text_w: width.saturating_sub(content_x) as usize,
            },
            SnapshotStyle {
                default_colors: colors,
                style,
                syntax_spans,
                lexical_fallback_enabled: use_lexical_fallback,
            },
            SnapshotOverlays {
                delimiter_highlights: &delimiter_highlights,
                active_scope_guides: &active_scope_guides,
                search_highlights: preview_highlights.as_ref().unwrap_or(search_highlights),
                substitution_colors,
                snippet_placeholders: if preview.is_some() {
                    &empty_snippets
                } else {
                    snippet_placeholders
                },
                diagnostic_lines: if preview.is_some() {
                    &empty_diagnostics
                } else {
                    diagnostic_lines
                },
                visual_selection: visual_selection.filter(|_| preview.is_none()),
                one_shot_highlight: one_shot_highlight.filter(|_| preview.is_none()),
                focused_lines,
            },
        )
    }) else {
        return Ok(());
    };
    result
}

pub(crate) fn visible_buffer_lines(
    buffer: &TextBuffer,
    first_line: usize,
    max_lines: usize,
) -> (usize, Vec<String>) {
    if max_lines == 0 {
        return (0, Vec::new());
    }

    let first_line = first_line.min(buffer.len_lines().saturating_sub(1));
    let end_line = first_line.saturating_add(max_lines).min(buffer.len_lines());
    (
        first_line,
        (first_line..end_line)
            .map(|line| buffer.line_string(line))
            .collect(),
    )
}

pub(crate) fn draw_snapshot_lines(
    window: &mut dyn Window,
    buffer: &redox_core::TextBuffer,
    snapshot: &crate::ui::render::RenderSnapshot,
    viewport: SnapshotViewport,
    appearance: SnapshotStyle<'_>,
    overlays: SnapshotOverlays<'_>,
) -> minui::Result<()> {
    let SnapshotViewport {
        content_x,
        scroll_x,
        text_w,
    } = viewport;
    let SnapshotStyle {
        default_colors,
        style,
        syntax_spans,
        lexical_fallback_enabled,
    } = appearance;
    let SnapshotOverlays {
        delimiter_highlights,
        active_scope_guides,
        search_highlights,
        substitution_colors,
        snippet_placeholders,
        diagnostic_lines,
        visual_selection,
        one_shot_highlight,
        focused_lines,
    } = overlays;

    let search_background = substitution_colors.map_or(style.search_match.bg, |colors| colors.bg);
    let search_foreground = substitution_colors
        .map(|colors| colors.fg)
        .or_else(|| (style.search_match.fg != Color::Transparent).then_some(style.search_match.fg));
    let color_column = visible_color_column(
        scroll_x,
        text_w,
        style.layout.color_column,
        style.theme.color_column,
    );
    // Lines outside the focused range draw every syntax role in the ghost colour.
    let ghost_colors = default_colors.with_colors(style.zen_ghost, default_colors.bg);
    let ghost_style = focused_lines.is_some().then(|| {
        let mut ghost = *style;
        for (_, syntax_style) in ghost.syntax_roles_mut() {
            *syntax_style = syntax_style.with_colors(ghost_colors.fg, ghost_colors.bg);
        }
        ghost
    });
    // One-shot highlights only show while no visual selection is active.
    let one_shot = one_shot_highlight.filter(|_| visual_selection.is_none());
    for (row, render_line) in snapshot.iter().enumerate() {
        let line_viewport = LineViewport {
            row: row as u16,
            column: content_x,
            scroll_x,
            width: text_w,
        };
        let line_idx = snapshot.first_line() + row;
        let visible_line = render_line.visible();
        let source_line = render_line.source();
        let is_focused = focused_lines
            .as_ref()
            .is_none_or(|lines| lines.contains(&line_idx));
        let (default_colors, style) = match &ghost_style {
            Some(ghost_style) if !is_focused => (ghost_colors, ghost_style),
            _ => (default_colors, style),
        };
        let fallback_line_spans = lexical_fallback_enabled
            .then(|| lexical_fallback_line_spans(source_line))
            .filter(|spans| !spans.is_empty());
        let syntax_line_spans = syntax_spans
            .and_then(|rows| rows.get(row))
            .or(fallback_line_spans.as_deref());
        let highlighted_chars = delimiter_highlights
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let filtered_delimiters;
        let highlighted_chars = if let Some(highlights) = search_highlights.get(&line_idx) {
            filtered_delimiters = highlighted_chars
                .iter()
                .copied()
                .filter(|column| {
                    let overlaps = |range: &std::ops::Range<usize>| {
                        *column >= range.start
                            && *column < range.end.max(range.start.saturating_add(1))
                    };
                    if substitution_colors.is_some() {
                        !highlights.ranges.iter().any(overlaps)
                    } else {
                        !highlights.active.as_ref().is_some_and(overlaps)
                    }
                })
                .collect::<Vec<_>>();
            filtered_delimiters.as_slice()
        } else {
            highlighted_chars
        };
        let visible_indent_guides = active_scope_guides
            .get(&line_idx)
            .filter(|_| substitution_colors.is_none())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let diagnostic_line = diagnostic_lines.get(&line_idx);
        let snippet_ranges = snippet_placeholders
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let has_search_highlights = search_highlights
            .get(&line_idx)
            .is_some_and(|highlights| !highlights.ranges.is_empty());
        let diagnostic_cells = diagnostic_line.map(|diagnostic| {
            selected_visible_cells(
                source_line,
                scroll_x,
                text_w,
                diagnostic.start_col,
                diagnostic
                    .end_col
                    .max(diagnostic.start_col.saturating_add(1)),
            )
        });
        let error_cells = diagnostic_cells
            .as_deref()
            .filter(|_| {
                diagnostic_line.is_some_and(|diagnostic| {
                    diagnostic.severity == crate::app::DiagnosticSeverity::Error
                })
            })
            .unwrap_or(&[]);
        let transient_selection = visual_selection
            .map(|(selection, mode)| (selection, mode, style.theme.selection_bg))
            .or_else(|| {
                one_shot_highlight
                    .map(|highlight| (highlight.selection, highlight.mode, default_colors.bg))
            });

        let selected_cells = transient_selection.and_then(|(selection, mode, _)| {
            transient_selection_cells(
                buffer,
                source_line,
                (selection, mode),
                line_idx,
                (scroll_x, text_w),
                one_shot,
            )
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
            continue;
        }

        let occupied_text_cells = if visible_indent_guides.is_empty() {
            Vec::new()
        } else {
            occupied_visible_cells(source_line, scroll_x, text_w)
        };
        let search_cells = search_highlights.get(&line_idx).map(|highlights| {
            search_highlight_cells(source_line, scroll_x, text_w, &highlights.ranges)
        });
        let active_search_cells = search_highlights
            .get(&line_idx)
            .and_then(|highlights| highlights.active.as_ref())
            .map(|range| {
                search_highlight_cells(source_line, scroll_x, text_w, std::slice::from_ref(range))
            });
        let mut decorations = LineDecorations {
            search_cells: search_cells.as_deref().unwrap_or(&[]),
            error_cells,
            current_cells: active_search_cells.as_deref().unwrap_or(&[]),
            search_style: style.search_match,
            current_style: style.search_current,
            error_style: style.error_range,
            yank_highlight: None,
            jump_highlight: None,
            delimiter_blink: None,
            undo_redo_highlight: None,
        };
        if let Some(highlight) = one_shot {
            add_one_shot_decorations(
                &mut decorations,
                highlight,
                selected_cells.as_deref(),
                style,
                default_colors,
            );
        }

        let active_search_layer = active_search_cells.as_deref().map(|cells| {
            (
                cells,
                style.search_current.bg,
                (style.search_current.fg != Color::Transparent).then_some(style.search_current.fg),
            )
        });
        let search_layer = search_cells
            .as_deref()
            .map(|cells| (cells, search_background, search_foreground));
        let diagnostic_layer =
            diagnostic_cells
                .as_deref()
                .zip(diagnostic_line)
                .map(|(cells, diagnostic)| {
                    (
                        cells,
                        style.diagnostic_inline.background(diagnostic.severity),
                        None,
                    )
                });
        let non_empty_syntax_spans = syntax_line_spans.filter(|spans| !spans.is_empty());

        // Layers paint first-match-wins, so the active search match sits on top. The selection
        // drops below the other layers while a jump fades and is left out during an undo flash.
        // `guide_backdrop` is what the indent guides draw over.
        let (highlights, guide_backdrop) = if let Some((_, _, selection_bg)) = transient_selection
            && let Some(selected_cells) = selected_cells.as_deref()
        {
            let selection_layer = (selected_cells, selection_bg, None);
            let selection_on_top =
                decorations.jump_highlight.is_none() && decorations.undo_redo_highlight.is_none();
            let layers = active_search_layer
                .into_iter()
                .chain(selection_on_top.then_some(selection_layer))
                .chain(search_layer)
                .chain(diagnostic_layer)
                .chain(
                    decorations
                        .jump_highlight
                        .is_some()
                        .then_some(selection_layer),
                )
                .collect::<Vec<_>>();
            (
                Some((layers, source_line.is_empty(), syntax_line_spans)),
                Some((selected_cells, selection_bg)),
            )
        } else if search_cells
            .as_ref()
            .is_some_and(|cells| cells.contains(&true))
        {
            let layers = active_search_layer
                .into_iter()
                .chain(search_layer)
                .chain(diagnostic_layer)
                .collect::<Vec<_>>();
            (
                Some((layers, source_line.is_empty(), syntax_line_spans)),
                active_search_cells
                    .as_deref()
                    .map(|cells| (cells, style.search_current.bg)),
            )
        } else {
            let highlights =
                diagnostic_layer.map(|layer| (vec![layer], false, non_empty_syntax_spans));
            (highlights, None)
        };

        if let Some((layers, empty_line, spans)) = highlights {
            draw_line_with_highlights(
                window,
                line_viewport,
                source_line,
                default_colors,
                LineHighlights {
                    color_column,
                    layers: &layers,
                    empty_line,
                    decorations,
                },
                style,
                spans,
            )?;
        } else if let Some(spans) = non_empty_syntax_spans {
            draw_line_with_syntax(
                window,
                line_viewport,
                source_line,
                default_colors,
                color_column,
                style,
                spans,
            )?;
        } else {
            draw_plain_line(
                window,
                line_viewport,
                source_line,
                default_colors,
                color_column,
            )?;
        }
        draw_indent_guides(
            window,
            (row as u16, content_x),
            visible_indent_guides,
            &occupied_text_cells,
            style,
            guide_backdrop,
            decorations,
        )?;
        draw_delimiter_highlights(
            window,
            line_viewport,
            source_line,
            (highlighted_chars, decorations),
            default_colors,
            style,
            syntax_line_spans,
        )?;
        draw_snippet_placeholders(
            window,
            line_viewport,
            source_line,
            snippet_ranges,
            style,
            decorations,
            syntax_line_spans,
        )?;
        if let Some(diagnostic) = diagnostic_line {
            draw_inline_diagnostic(window, line_viewport, source_line, style, diagnostic)?;
        }
    }

    Ok(())
}

/// The cells a visual selection or one-shot highlight covers on `line_idx`. `one_shot` is only
/// set when no visual selection is active.
pub(crate) fn transient_selection_cells(
    buffer: &redox_core::TextBuffer,
    source_line: &str,
    (selection, mode): (redox_core::Selection, redox_core::VisualModeKind),
    line_idx: usize,
    (scroll_x, text_w): (usize, usize),
    one_shot: Option<OneShotHighlight>,
) -> Option<Vec<bool>> {
    match one_shot.map(|highlight| highlight.kind) {
        Some(HighlightKind::Jump) => {
            return (selection.cursor.line == line_idx).then(|| vec![true; text_w]);
        }
        Some(HighlightKind::UndoRedo)
            if selection.anchor == selection.cursor
                && selection.cursor.line == line_idx
                && selection.cursor.col >= source_line.chars().count() =>
        {
            return Some(search_highlight_cells(
                source_line,
                scroll_x,
                text_w,
                std::slice::from_ref(&(selection.cursor.col..selection.cursor.col)),
            ));
        }
        _ => {}
    }
    visual_selection_visible_cells(
        buffer,
        source_line,
        selection,
        mode,
        line_idx,
        scroll_x,
        text_w,
    )
}

/// Adds the fading decoration for a one-shot highlight over `selected_cells`.
pub(crate) fn add_one_shot_decorations<'a>(
    decorations: &mut LineDecorations<'a>,
    highlight: OneShotHighlight,
    selected_cells: Option<&'a [bool]>,
    style: &UiStyle,
    default_colors: TextStyle,
) {
    let fade = HighlightFade::new(highlight.progress(), style.editor_text.fg);
    match highlight.kind {
        HighlightKind::Yank => {
            decorations.yank_highlight = selected_cells.and_then(|cells| {
                YankHighlight::new(cells, highlight.progress(), default_colors.colors())
            });
        }
        HighlightKind::Jump => decorations.jump_highlight = selected_cells.map(|_| fade),
        HighlightKind::Delimiter => {
            decorations.delimiter_blink = selected_cells
                .and_then(|cells| cells.iter().position(|selected| *selected))
                .map(|column| (column, fade));
        }
        HighlightKind::UndoRedo => {
            decorations.undo_redo_highlight = selected_cells.map(|cells| (cells, fade));
        }
    }
}

pub(crate) fn should_use_lexical_fallback(
    syntax_language: Option<crate::ui::syntax::SyntaxLanguage>,
) -> bool {
    syntax_language.is_none()
}
