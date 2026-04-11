use super::*;
use redox_core::{
    BufferLoadPhase, DelimiterKind, TextObjectKind, TextObjectScope, TextObjectSpec,
    VisualModeKind, motion::Motion,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::input::{InputAction, InputMode, InsertKind, OperatorTarget, TextObjectOperator};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;

fn temp_file_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_nanos();
    std::env::temp_dir().join(format!("redox_state_test_{tag}_{nanos}.txt"))
}

fn state_with_text(path: PathBuf, text: &str) -> EditorState {
    fs::write(&path, text).expect("failed to write test file");
    let session = EditorSession::open_initial_file(&path).expect("failed to open session");
    EditorState::new(session)
}

fn large_text(lines: usize) -> String {
    let mut out = String::new();
    for i in 0..lines {
        out.push_str(&format!("line-{i:05} abcdefghijklmnopqrstuvwxyz\n"));
    }
    out
}

fn run_command(state: &mut EditorState, cmd: &str) {
    state.mode = EditorMode::Command;
    state.command_line = cmd.to_string();
    state.apply_input(InputAction::CommandEnter, 80, 24);
}

#[test]
fn normal_mode_paste_inserts_text_and_marks_dirty() {
    let path = temp_file_path("paste_normal");
    let mut state = state_with_text(path.clone(), "hello");

    state.apply_input(InputAction::Paste(" world".to_string()), 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), " worldhello");
    assert!(state.session.active_meta().dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn invalidate_render_caches_keeps_analysis_cache_until_worker_result() {
    let mut view = BufferViewState::default();
    let before = TextBuffer::from_str("{ alpha }");
    let after = TextBuffer::from_str("plain text");

    view.delimiter_pair_cache
        .install(crate::ui::overlays::compute_delimiter_analysis(&before));
    assert_eq!(view.delimiter_pair_cache.get().expect("analysis").len(), 1);

    let previous_version = view.analysis_version;
    view.invalidate_render_caches();

    assert_ne!(view.analysis_version, previous_version);
    assert_eq!(view.delimiter_pair_cache.get().expect("analysis").len(), 1);

    view.delimiter_pair_cache
        .install(crate::ui::overlays::compute_delimiter_analysis(&after));
    assert!(
        view.delimiter_pair_cache
            .get()
            .expect("analysis")
            .is_empty()
    );
}

#[test]
fn stale_analysis_results_are_dropped() {
    let path = temp_file_path("stale_analysis_result");
    let mut state = state_with_text(path.clone(), "plain text");
    let active_id = state.session.active_id();
    let current_version = {
        let view = state.views.entry(active_id).or_default();
        view.invalidate_render_caches();
        view.analysis_version
    };

    state.apply_analysis_result(analysis::AnalysisResult {
        buffer_id: active_id,
        version: current_version,
        syntax_cache: None,
        delimiter_analysis: crate::ui::overlays::compute_delimiter_analysis(
            state.session.active_buffer(),
        ),
    });
    state.apply_analysis_result(analysis::AnalysisResult {
        buffer_id: active_id,
        version: current_version.saturating_sub(1),
        syntax_cache: None,
        delimiter_analysis: crate::ui::overlays::compute_delimiter_analysis(&TextBuffer::from_str(
            "{ stale }",
        )),
    });

    let view = state
        .views
        .get_mut(&active_id)
        .expect("missing active view");
    assert!(
        view.delimiter_pair_cache
            .get()
            .expect("analysis")
            .is_empty()
    );

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_system_clipboard_paste_matches_p_semantics() {
    let path = temp_file_path("paste_system_clipboard_normal");
    let mut state = state_with_text(path.clone(), "abcd");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(
        InputAction::PasteSystemClipboardText("XY".to_string()),
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "abXYcd");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 4));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_system_clipboard_paste_normalizes_crlf() {
    let path = temp_file_path("paste_system_clipboard_crlf");
    let mut state = state_with_text(path.clone(), "one\ntwo\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(
        InputAction::PasteSystemClipboardText("X\r\nY".to_string()),
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "onX\nYe\ntwo\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_system_clipboard_paste_strips_control_characters() {
    let path = temp_file_path("paste_system_clipboard_controls");
    let mut state = state_with_text(path.clone(), "abcd");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(
        InputAction::PasteSystemClipboardText("X\u{1b}Y\u{0008}Z".to_string()),
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "abXYZcd");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 5));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_line_start_motions_distinguish_zero_underscore_and_dollar() {
    let path = temp_file_path("line_start_motions");
    let mut state = state_with_text(path.clone(), "\t  alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 5);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::LineStart,
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 5);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::LineFirstNonWhitespace,
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 3));

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::LineEnd,
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 7));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_operate_motion_supports_delete_to_line_end_and_start() {
    let path = temp_file_path("operate_motion_delete_line");
    let mut state = state_with_text(path.clone(), "  alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 4);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::LineEnd,
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "  al\n");
    assert_eq!(state.private_register, "pha");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 3));

    let mut state = state_with_text(path.clone(), "  alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 5);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::LineStart,
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "ha\n");
    assert_eq!(state.private_register, "  alp");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_operate_motion_line_end_count_spans_multiple_lines() {
    let path = temp_file_path("operate_motion_delete_line_end_count");
    let mut state = state_with_text(path.clone(), "alpha\nbeta\ngamma\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 2);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::LineEnd,
                count: 2,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "al\ngamma\n");
    assert_eq!(state.private_register, "pha\nbeta");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_find_and_till_char_move_to_expected_columns() {
    let path = temp_file_path("find_till_motion");
    let mut state = state_with_text(path.clone(), "alpha beta alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::FindChar('b'),
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 6));

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::TillChar('b'),
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 5));

    let _ = fs::remove_file(path);
}

#[test]
fn operator_find_and_till_char_apply_expected_ranges() {
    let path = temp_file_path("operator_find_till");
    let mut state = state_with_text(path.clone(), "abc def ghi\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::TillChar('g'),
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "ghi\n");
    assert_eq!(state.private_register, "abc def ");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let mut state = state_with_text(path.clone(), "abc def ghi\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Change,
            target: OperatorTarget::Motion {
                motion: Motion::FindChar('g'),
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "hi\n");
    assert_eq!(state.private_register, "abc def g");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn operator_till_char_matches_vim_style_dtt_behaviour() {
    let path = temp_file_path("operator_till_char_dtt");
    let mut state = state_with_text(path.clone(), "formatting\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::TillChar('t'),
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "ftting\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_operate_motion_first_non_whitespace_count_spans_multiple_lines() {
    let path = temp_file_path("operate_motion_delete_first_non_ws_count");
    let mut state = state_with_text(path.clone(), "alpha\n  beta\n\tgamma\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 2);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::Motion {
                motion: Motion::LineFirstNonWhitespace,
                count: 3,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "algamma\n");
    assert_eq!(state.private_register, "pha\n  beta\n\t");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 2));

    let _ = fs::remove_file(path);
}

#[test]
fn slash_search_caches_matches_and_ctrl_n_ctrl_p_repeat_them() {
    let path = temp_file_path("slash_search_repeat");
    let mut state = state_with_text(path.clone(), "alpha beta alpha gamma alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "alpha".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);

    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 11));
    assert_eq!(
        state.active_search_highlight_ranges(0, 1).get(&0).cloned(),
        Some(vec![0..5, 11..16, 23..28])
    );

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 23));

    state.apply_input(InputAction::RepeatSearch { forward: false }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 11));

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 23));
    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn till_char_search_repeat_tracks_the_actual_matched_character() {
    let path = temp_file_path("till_char_search_repeat_target");
    let mut state = state_with_text(path.clone(), "aabac\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::TillChar('a'),
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 2));

    let _ = fs::remove_file(path);
}

#[test]
fn repeat_search_reports_when_no_other_instances_exist() {
    let path = temp_file_path("slash_search_single");
    let mut state = state_with_text(path.clone(), "alpha beta gamma\n");

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "beta".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 6));
    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(
        state.status_msg.as_deref(),
        Some("no other pattern instances")
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 6));

    let _ = fs::remove_file(path);
}

#[test]
fn repeat_search_rediscovers_single_cached_match_after_hiding_highlights() {
    let path = temp_file_path("slash_search_single_rediscover");
    let mut state = state_with_text(path.clone(), "alpha beta gamma\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "beta".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 6));
    state.apply_input(InputAction::ClearSearch, 80, 24);
    assert!(state.active_search_highlight_ranges(0, 1).is_empty());

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 6));
    assert!(state.active_search_highlight_ranges(0, 1).contains_key(&0));
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn manual_motion_and_escape_hide_search_highlights_but_keep_cached_query() {
    let path = temp_file_path("slash_search_hide_highlights");
    let mut state = state_with_text(path.clone(), "alpha beta alpha gamma alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "alpha".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 11));
    assert!(state.active_search_highlight_ranges(0, 1).contains_key(&0));

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Left,
            count: 10,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));
    assert!(state.active_search_highlight_ranges(0, 1).is_empty());

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 11));
    assert!(state.active_search_highlight_ranges(0, 1).contains_key(&0));

    state.apply_input(InputAction::ClearSearch, 80, 24);
    assert!(state.active_search_highlight_ranges(0, 1).is_empty());

    let _ = fs::remove_file(path);
}

#[test]
fn paging_and_centering_hide_search_highlights_but_keep_cached_query() {
    let path = temp_file_path("slash_search_hide_highlights_on_paging");
    let text = large_text(120);
    let mut state = state_with_text(path.clone(), &text);
    let viewport_height_rows = 8usize;

    state.apply_input(InputAction::EnterSearch, 80, viewport_height_rows);
    for ch in "line".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, viewport_height_rows);
    }
    state.apply_input(InputAction::SearchEnter, 80, viewport_height_rows);

    assert!(!state.active_search_highlight_ranges(0, 8).is_empty());

    state.apply_input(InputAction::ViewportDownCenter, 80, viewport_height_rows);
    assert!(state.active_search_highlight_ranges(0, 8).is_empty());

    state.apply_input(
        InputAction::RepeatSearch { forward: true },
        80,
        viewport_height_rows,
    );
    assert!(!state.active_search_highlight_ranges(0, 8).is_empty());

    state.apply_input(InputAction::ViewportUpCenter, 80, viewport_height_rows);
    assert!(state.active_search_highlight_ranges(0, 8).is_empty());

    state.apply_input(
        InputAction::RepeatSearch { forward: true },
        80,
        viewport_height_rows,
    );
    assert!(!state.active_search_highlight_ranges(0, 8).is_empty());

    state.apply_input(InputAction::CenterCursorLine, 80, viewport_height_rows);
    assert!(state.active_search_highlight_ranges(0, 8).is_empty());

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_shift_i_enters_insert_at_first_non_whitespace() {
    let path = temp_file_path("shift_i_first_non_whitespace");
    let mut state = state_with_text(path.clone(), "\t  alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 6);

    state.apply_input(
        InputAction::EnterInsert(InsertKind::InsertLineStart),
        80,
        24,
    );

    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 3));

    let _ = fs::remove_file(path);
}

#[test]
fn insert_mode_repeated_same_line_paste_keeps_cursor_and_horizontal_scroll_in_sync() {
    let path = temp_file_path("insert_mode_repeated_paste_scroll");
    let mut state = state_with_text(path.clone(), "");
    let chunk = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefgh";

    state.apply_input(InputAction::SetMode(InputMode::Insert), 80, 24);
    state.apply_input(InputAction::Paste(chunk.to_string()), 80, 24);
    state.apply_input(InputAction::Paste(chunk.to_string()), 80, 24);

    let id = state.session.active_id();
    let view = state.views.get(&id).expect("missing active view");
    let buffer = state.session.active_buffer();
    let spec = view.cursor.clone().cursor_spec(buffer, 80, 23);

    assert_eq!(
        state.session.active_buffer().to_string(),
        format!("{chunk}{chunk}")
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, chunk.len() * 2));
    assert_eq!(view.cursor.scroll_x_cells, chunk.len() * 2 - 79);
    assert!(spec.visible);
    assert_eq!(spec.x, 79);

    let _ = fs::remove_file(path);
}

#[test]
fn exiting_insert_mode_after_tab_keeps_cursor_after_tab() {
    let path = temp_file_path("insert_exit_after_tab");
    let mut state = state_with_text(path.clone(), "\talpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::Insert), 80, 24);
    state.apply_input(InputAction::SetMode(InputMode::Normal), 80, 24);

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_replace_char_replaces_tab_as_single_logical_character() {
    let path = temp_file_path("replace_tab_normal");
    let mut state = state_with_text(path.clone(), "\tab\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::ReplaceChar('x'), 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "xab\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_char_replace_replaces_entire_selection_and_normalizes_mode() {
    let path = temp_file_path("replace_visual_char");
    let mut state = state_with_text(path.clone(), "\tab\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::ReplaceChar('x'), 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "xxb\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_replace_preserves_block_shape_when_replacing_tabs() {
    let path = temp_file_path("replace_visual_block_tab");
    let mut state = state_with_text(path.clone(), "\tab\n\tcd\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::ReplaceChar('x'), 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "xxb\nxxd\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_replace_preserves_line_structure() {
    let path = temp_file_path("replace_visual_line");
    let mut state = state_with_text(path.clone(), "ab\ncde\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::ReplaceChar('x'), 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "xx\nxxx\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_u_undoes_and_ctrl_r_redoes_last_edit() {
    let path = temp_file_path("undo_redo_basic");
    let mut state = state_with_text(path.clone(), "hello");

    state.apply_input(InputAction::Paste(" world".to_string()), 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), " worldhello");

    state.apply_input(InputAction::Undo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "hello");

    state.apply_input(InputAction::Redo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), " worldhello");

    let _ = fs::remove_file(path);
}

#[test]
fn redo_stack_is_cleared_after_new_edit_post_undo() {
    let path = temp_file_path("redo_cleared_after_new_edit");
    let mut state = state_with_text(path.clone(), "hello");

    state.apply_input(InputAction::Paste("A".to_string()), 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "Ahello");

    state.apply_input(InputAction::Undo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "hello");

    state.apply_input(InputAction::Paste("B".to_string()), 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "Bhello");

    state.apply_input(InputAction::Redo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "Bhello");

    let _ = fs::remove_file(path);
}

#[test]
fn insert_mode_typing_is_coalesced_into_single_undo_step() {
    let path = temp_file_path("insert_mode_undo_coalesce");
    let mut state = state_with_text(path.clone(), "hello");

    state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);
    state.apply_input(InputAction::InsertChar('a'), 80, 24);
    state.apply_input(InputAction::InsertChar('b'), 80, 24);
    state.apply_input(InputAction::InsertChar('c'), 80, 24);
    state.apply_input(InputAction::SetMode(InputMode::Normal), 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "abchello");

    state.apply_input(InputAction::Undo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "hello");

    state.apply_input(InputAction::Redo, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "abchello");

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_cursor_stops_on_last_character_of_non_empty_line() {
    let path = temp_file_path("normal_mode_last_char");
    let mut state = state_with_text(path.clone(), "abc\n");

    for _ in 0..5 {
        state.apply_input(
            InputAction::Motion {
                motion: Motion::Right,
                count: 1,
            },
            80,
            24,
        );
    }

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 2));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_cursor_can_stay_at_zero_on_empty_line() {
    let path = temp_file_path("normal_mode_empty_line");
    let mut state = state_with_text(path.clone(), "");

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn switching_buffers_preserves_cursor_and_scroll_state() {
    let path_a = temp_file_path("switch_preserve_a");
    let path_b = temp_file_path("switch_preserve_b");
    let mut state = state_with_text(path_a.clone(), "aaaa\nbbbb\n");
    fs::write(&path_b, "cccc\ndddd\n").expect("failed to write test file");

    let id_a = state.session.active_id();
    {
        let view = state
            .views
            .get_mut(&id_a)
            .expect("missing view for buffer A");
        view.cursor.cursor = Pos::new(1, 2);
        view.cursor.scroll_x_cells = 4;
        view.cursor.scroll_y_lines = 1;
    }

    run_command(&mut state, &format!("e {}", path_b.display()));
    let id_b = state.session.active_id();

    {
        let view = state
            .views
            .get_mut(&id_b)
            .expect("missing view for buffer B");
        view.cursor.cursor = Pos::new(0, 3);
        view.cursor.scroll_x_cells = 7;
        view.cursor.scroll_y_lines = 0;
    }

    run_command(&mut state, "bp");

    assert_eq!(state.session.active_id(), id_a);
    let view_a = state.views.get(&id_a).expect("missing view for buffer A");
    assert_eq!(view_a.cursor.cursor, Pos::new(1, 2));
    assert_eq!(view_a.cursor.scroll_x_cells, 4);
    assert_eq!(view_a.cursor.scroll_y_lines, 1);

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn command_q_does_not_quit_when_hidden_buffer_is_dirty() {
    let path_a = temp_file_path("q_hidden_dirty_a");
    let path_b = temp_file_path("q_hidden_dirty_b");
    let mut state = state_with_text(path_a.clone(), "aaa");
    fs::write(&path_b, "bbb").expect("failed to write test file");

    run_command(&mut state, &format!("e {}", path_b.display()));
    run_command(&mut state, "bp");
    state.apply_input(InputAction::Paste("x".to_string()), 80, 24);
    run_command(&mut state, "bn");

    run_command(&mut state, "q");

    assert!(!state.should_quit);
    let msg = state.status_msg.as_deref().expect("missing quit warning");
    let leaf_a = path_a
        .file_name()
        .and_then(|name| name.to_str())
        .expect("path should have a file name");
    assert!(msg.contains("unsaved changes in"));
    assert!(msg.contains(leaf_a));
    assert!(msg.contains("use :q! to quit"));

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn command_w_writes_active_buffer_only() {
    let path_a = temp_file_path("write_active_a");
    let path_b = temp_file_path("write_active_b");
    let mut state = state_with_text(path_a.clone(), "alpha");
    fs::write(&path_b, "bravo").expect("failed to write test file");

    run_command(&mut state, &format!("e {}", path_b.display()));
    let id_b = state.session.active_id();

    state.apply_input(InputAction::Paste("Z".to_string()), 80, 24);
    assert!(state.session.meta(id_b).expect("missing meta").dirty);

    run_command(&mut state, "bp");
    run_command(&mut state, "w");

    assert!(state.session.meta(id_b).expect("missing meta").dirty);
    let on_disk_b = fs::read_to_string(&path_b).expect("failed to read file B");
    assert_eq!(on_disk_b, "bravo");

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn command_ls_populates_compact_status_summary() {
    let path_a = temp_file_path("ls_a");
    let path_b = temp_file_path("ls_b");
    let mut state = state_with_text(path_a.clone(), "alpha");
    fs::write(&path_b, "bravo").expect("failed to write test file");

    state.apply_input(InputAction::Paste("!".to_string()), 80, 24);
    run_command(&mut state, &format!("e {}", path_b.display()));
    run_command(&mut state, "ls");

    let msg = state.status_msg.as_deref().expect("missing ls status");
    assert!(msg.contains("|"));
    assert!(msg.contains("%"));
    assert!(msg.contains("+"));
    assert!(msg.contains(" | "));
    for summary in state.session.summaries() {
        assert!(msg.contains(&summary.display_name));
    }

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn command_rain_captures_and_stop_clears_animation_state() {
    let path = temp_file_path("rain_mode");
    let mut state = state_with_text(path.clone(), "let rain = true;\n");

    run_command(&mut state, "rain");

    assert!(state.rain_is_active());
    assert!(state.rain_pending_start);
    assert!(state.rain_animation.is_none());
    assert_eq!(state.status_msg.as_deref(), Some("making it rain"));

    state.ensure_rain_animation(
        20,
        6,
        minui::ColorPair::new(minui::Color::White, minui::Color::Black),
        crate::ui::UiStyle::default(),
    );

    assert!(state.rain_animation.is_some());
    assert!(!state.rain_pending_start);

    state.stop_rain_animation();

    assert!(!state.rain_is_active());
    assert!(state.rain_animation.is_none());
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_ls_status_is_cleared_on_next_input() {
    let path = temp_file_path("ls_ephemeral");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "ls");
    assert!(state.status_msg.is_some());

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );

    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_write_status_is_cleared_on_next_input() {
    let path = temp_file_path("write_status_clears");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "w");
    assert_eq!(state.status_msg.as_deref(), Some("written"));

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );

    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_e_uses_trimmed_remainder_as_path() {
    let path_a = temp_file_path("e_trimmed_a");
    let path_b = temp_file_path("e_trimmed_b");
    let mut state = state_with_text(path_a.clone(), "alpha");
    fs::write(&path_b, "bravo").expect("failed to write test file");

    run_command(&mut state, &format!("e    {}", path_b.display()));

    assert_eq!(state.session.active_buffer().to_string(), "bravo");

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn dirty_tracking_clears_after_reverting_to_original_content() {
    let path = temp_file_path("dirty_revert_state");
    let mut state = state_with_text(path.clone(), "hello");

    state.apply_input(InputAction::Paste("x".to_string()), 80, 24);
    assert!(state.session.active_meta().dirty);

    state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);
    state.apply_input(InputAction::Backspace, 80, 24);
    assert!(!state.session.active_meta().dirty);

    run_command(&mut state, "q");
    assert!(state.should_quit);

    let _ = fs::remove_file(path);
}

#[test]
fn unknown_command_sets_status_message() {
    let path = temp_file_path("unknown_command");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "zzzz");

    assert_eq!(state.status_msg.as_deref(), Some("unknown command: zzzz"));

    let _ = fs::remove_file(path);
}

#[test]
fn explorer_command_opens_ui_buffer() {
    let path = temp_file_path("explorer_open");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "explorer");

    assert!(state.explorer_popup().is_some());
    assert!(state.active_display_name().contains("[explorer]"));
    assert!(
        state
            .session
            .active_buffer()
            .to_string()
            .lines()
            .any(|line| line == "../")
    );
    let popup = state
        .explorer_popup()
        .expect("explorer popup should be active");
    assert!(popup.title.starts_with('~'));
    assert!(popup.title.ends_with('/'));

    let _ = fs::remove_file(path);
}

#[test]
fn explorer_write_applies_rename_and_create() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_a = dir.join("a.txt");
    let file_open = dir.join("open.txt");
    fs::write(&file_a, "a").expect("failed to write fixture");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);

    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nrenamed.txt\ncreated.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    assert!(dir.join("renamed.txt").exists());
    assert!(dir.join("created.txt").exists());
    assert!(!dir.join("a.txt").exists());

    let _ = fs::remove_file(dir.join("renamed.txt"));
    let _ = fs::remove_file(dir.join("created.txt"));
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_insert_in_middle_preserves_existing_file_contents() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_insert_middle_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_a = dir.join("a.txt");
    let file_b = dir.join("b.txt");
    fs::write(&file_a, "alpha").expect("failed to write fixture");
    fs::write(&file_b, "beta").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_a).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\na.txt\nnew.txt\nb.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    assert_eq!(
        fs::read_to_string(&file_a).expect("failed to read a.txt"),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(&file_b).expect("failed to read b.txt"),
        "beta"
    );
    assert_eq!(
        fs::read_to_string(dir.join("new.txt")).expect("failed to read new.txt"),
        ""
    );

    let _ = fs::remove_file(file_a);
    let _ = fs::remove_file(file_b);
    let _ = fs::remove_file(dir.join("new.txt"));
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_requires_confirmation_for_deletes() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_confirm_delete_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_keep = dir.join("a_keep.txt");
    let file_delete = dir.join("z_delete.txt");
    fs::write(&file_keep, "keep").expect("failed to write fixture");
    fs::write(&file_delete, "delete").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_keep).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\na_keep.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    assert!(file_delete.exists());
    let msg = state
        .status_msg
        .as_deref()
        .expect("missing confirmation prompt");
    assert!(msg.contains("confirm deletion of 1 entry"));
    assert!(!state.status_msg_clear_on_input);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    assert!(
        state
            .status_msg
            .as_deref()
            .is_some_and(|msg| msg.contains("confirm deletion of 1 entry"))
    );

    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);
    assert!(!file_delete.exists());
    assert_eq!(state.status_msg.as_deref(), Some("written"));

    let _ = fs::remove_file(file_keep);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_recursively_deletes_non_empty_directory_after_confirmation() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_confirm_delete_dir_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_keep = dir.join("a_keep.txt");
    let doomed_dir = dir.join("nested");
    fs::write(&file_keep, "keep").expect("failed to write fixture");
    fs::create_dir(&doomed_dir).expect("failed to create nested fixture");
    fs::write(doomed_dir.join("child.txt"), "child").expect("failed to write child fixture");

    let session = EditorSession::open_initial_file(&file_keep).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\na_keep.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    assert!(doomed_dir.exists());
    let msg = state
        .status_msg
        .as_deref()
        .expect("missing confirmation prompt");
    assert!(msg.contains("confirm deletion of 1 entry"));

    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);
    assert!(!doomed_dir.exists());
    assert_eq!(state.status_msg.as_deref(), Some("written"));

    let _ = fs::remove_file(file_keep);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_preserves_cursor_line_when_still_in_range() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_cursor_preserve_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    fs::write(dir.join("a.txt"), "a").expect("failed to write fixture");
    fs::write(dir.join("b.txt"), "b").expect("failed to write fixture");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line = 2;

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\na.txt\nrenamed.txt\nopen.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    let cursor_line = state
        .views
        .get(&id)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line;
    assert_eq!(cursor_line, 2);

    let _ = fs::remove_file(dir.join("a.txt"));
    let _ = fs::remove_file(dir.join("renamed.txt"));
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_clamps_cursor_to_bottom_when_lines_are_removed() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_cursor_clamp_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    fs::write(dir.join("a.txt"), "a").expect("failed to write fixture");
    fs::write(dir.join("b.txt"), "b").expect("failed to write fixture");
    fs::write(dir.join("c.txt"), "c").expect("failed to write fixture");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line = 4;

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\na.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);

    let cursor_line = state
        .views
        .get(&id)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line;
    assert_eq!(cursor_line, 1);

    let _ = fs::remove_file(dir.join("a.txt"));
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_rename_updates_hidden_buffer_path_for_bprev() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_hidden_rename_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let current = dir.join("current.txt");
    let victim = dir.join("victim.txt");
    let renamed = dir.join("renamed.txt");
    fs::write(&current, "current").expect("failed to write current fixture");
    fs::write(&victim, "victim").expect("failed to write victim fixture");

    let session = EditorSession::open_initial_file(&current).expect("failed to open session");
    let mut state = EditorState::new(session);
    let current_id = state.session.active_id();

    run_command(&mut state, &format!("e {}", victim.display()));
    let victim_id = state.session.active_id();
    run_command(&mut state, "bp");
    assert_eq!(state.session.active_id(), current_id);

    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\ncurrent.txt\nrenamed.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    run_command(&mut state, "q");

    run_command(&mut state, "bp");
    assert_eq!(state.session.active_id(), victim_id);
    let renamed = std::fs::canonicalize(&renamed).expect("renamed file should exist");
    assert_eq!(state.session.active_meta().path.as_ref(), Some(&renamed));
    assert_eq!(state.session.summaries().len(), 2);

    run_command(&mut state, &format!("e {}", renamed.display()));
    assert_eq!(state.session.active_id(), victim_id);
    assert_eq!(state.session.summaries().len(), 2);

    let _ = fs::remove_file(current);
    let _ = fs::remove_file(renamed);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_delete_removes_hidden_buffer_from_mru() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_hidden_delete_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let current = dir.join("current.txt");
    let doomed = dir.join("doomed.txt");
    fs::write(&current, "current").expect("failed to write current fixture");
    fs::write(&doomed, "doomed").expect("failed to write doomed fixture");

    let session = EditorSession::open_initial_file(&current).expect("failed to open session");
    let mut state = EditorState::new(session);
    let current_id = state.session.active_id();

    run_command(&mut state, &format!("e {}", doomed.display()));
    let doomed_id = state.session.active_id();
    run_command(&mut state, "bp");
    assert_eq!(state.session.active_id(), current_id);

    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\ncurrent.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);
    run_command(&mut state, "q");

    assert_eq!(state.session.summaries().len(), 1);
    assert!(state.session.meta(doomed_id).is_none());

    run_command(&mut state, "bp");
    assert_eq!(state.session.active_id(), current_id);
    assert_eq!(state.status_msg.as_deref(), Some("only one buffer"));

    let _ = fs::remove_file(current);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_delete_of_return_target_creates_placeholder_buffer() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_delete_return_target_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let doomed = dir.join("doomed.txt");
    fs::write(&doomed, "doomed").expect("failed to write doomed fixture");

    let session = EditorSession::open_initial_file(&doomed).expect("failed to open session");
    let mut state = EditorState::new(session);

    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");
    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);

    assert!(state.explorer_popup().is_some());
    assert!(state.explorer_background_is_placeholder_blank());

    run_command(&mut state, "q");
    assert!(state.should_quit);

    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_q_closes_surface_buffer_only() {
    let path = temp_file_path("explorer_q_close");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "explorer");
    assert!(state.explorer_popup().is_some());

    run_command(&mut state, "q");

    assert!(!state.should_quit);
    assert!(state.explorer_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn explorer_q_from_directory_launch_quits_in_one_step() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_single_q_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    fs::write(dir.join("a.txt"), "alpha").expect("failed to write fixture");

    let session = EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
    let mut state = EditorState::new(session);
    state
        .open_explorer_at_path(dir.clone())
        .expect("failed to open explorer");
    assert!(state.explorer_popup().is_some());

    run_command(&mut state, "q");

    assert!(state.should_quit);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn explorer_open_at_dot_resolves_title_to_real_directory_path() {
    let session = EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
    let mut state = EditorState::new(session);

    state
        .open_explorer_at_path(PathBuf::from("."))
        .expect("failed to open explorer at dot");
    let popup = state
        .explorer_popup()
        .expect("explorer popup should be active");

    assert!(!popup.title.starts_with("~./"));
    assert!(popup.title.ends_with('/'));
}

#[test]
fn explorer_directory_launch_marks_background_as_placeholder_blank() {
    let session = EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
    let mut state = EditorState::new(session);

    state
        .open_explorer_at_path(PathBuf::from("."))
        .expect("failed to open explorer at dot");

    assert!(state.explorer_background_is_placeholder_blank());
}

#[test]
fn explorer_command_toggles_visibility() {
    let path = temp_file_path("explorer_toggle");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "explorer");
    assert!(state.explorer_popup().is_some());

    state.apply_input(InputAction::OpenExplorer, 80, 24);
    assert!(state.explorer_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn about_command_opens_ui_buffer() {
    let path = temp_file_path("about_open");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "about");

    let popup = state.about_popup().expect("about popup should be open");
    assert_eq!(popup.title, "about");

    let _ = fs::remove_file(path);
}

#[test]
fn about_command_toggles_visibility() {
    let path = temp_file_path("about_toggle");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "about");
    assert!(state.about_popup().is_some());

    run_command(&mut state, "about");
    assert!(state.about_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn about_q_closes_surface_buffer_only() {
    let path = temp_file_path("about_q_close");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "about");
    assert!(state.about_popup().is_some());

    run_command(&mut state, "q");

    assert!(!state.should_quit);
    assert!(state.about_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn about_q_quits_from_empty_startup_buffer() {
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);

    run_command(&mut state, "about");
    assert!(state.about_popup().is_some());

    run_command(&mut state, "q");

    assert!(state.should_quit);
    assert!(state.about_popup().is_none());
}

#[test]
fn about_escape_key_closes_surface_buffer_only() {
    let path = temp_file_path("about_escape_key_close");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "about");
    assert!(state.about_popup().is_some());

    assert!(state.handle_normal_mode_escape_on_surface());

    assert!(!state.should_quit);
    assert!(state.about_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn about_escape_key_quits_from_empty_startup_buffer() {
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);

    run_command(&mut state, "about");
    assert!(state.about_popup().is_some());

    assert!(state.handle_normal_mode_escape_on_surface());

    assert!(state.should_quit);
    assert!(state.about_popup().is_none());
}

#[test]
fn explorer_escape_key_closes_surface_buffer_only() {
    let path = temp_file_path("explorer_escape_key_close");
    let mut state = state_with_text(path.clone(), "alpha");
    let return_to = state.session.active_id();

    run_command(&mut state, "explorer");
    assert!(state.explorer_popup().is_some());

    assert!(state.handle_normal_mode_escape_on_surface());

    assert!(!state.should_quit);
    assert!(state.explorer_popup().is_none());
    assert_eq!(state.session.active_id(), return_to);

    let _ = fs::remove_file(path);
}

#[test]
fn explorer_opens_from_startup_about_without_quitting() {
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);
    state.command_open_about();
    assert!(state.about_popup().is_some());

    run_command(&mut state, "explorer");

    assert!(!state.should_quit);
    assert!(state.about_popup().is_none());
    assert!(state.explorer_popup().is_some());
    assert!(state.explorer_background_is_placeholder_blank());
}

#[test]
fn explorer_enter_opens_file_and_closes_explorer() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_enter_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_a = dir.join("a.txt");
    let file_open = dir.join("open.txt");
    fs::write(&file_a, "aaa").expect("failed to write fixture");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    {
        let text = state.session.active_buffer().to_string();
        let target_line = text
            .lines()
            .position(|line| line == "a.txt")
            .expect("a.txt missing from explorer listing");
        let id = state.session.active_id();
        state
            .views
            .get_mut(&id)
            .expect("missing explorer view")
            .cursor
            .cursor
            .line = target_line;
    }

    state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

    assert!(state.explorer_popup().is_none());
    assert_eq!(state.session.active_buffer().to_string(), "aaa");

    let _ = fs::remove_file(file_a);
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_opens_with_cursor_on_current_file() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_cursor_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_a = dir.join("a.txt");
    let file_open = dir.join("open.txt");
    fs::write(&file_a, "aaa").expect("failed to write fixture");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    let line_idx = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "open.txt")
        .expect("open.txt missing from explorer");
    let active = state.session.active_id();
    let cursor_line = state
        .views
        .get(&active)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line;
    assert_eq!(cursor_line, line_idx);

    let _ = fs::remove_file(file_a);
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_motion_uses_no_scrolloff_and_clamps_window_scroll() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_scrolloff_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");
    for i in 0..12 {
        fs::write(dir.join(format!("f{i}.txt")), "x").expect("failed to write fixture");
    }

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line = 0;

    // Small viewport (text height 5) used to reproduce prior scrolloff behavior.
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        6,
    );
    assert_eq!(
        state
            .views
            .get(&id)
            .expect("missing explorer view")
            .cursor
            .scroll_y_lines,
        0
    );

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 999,
        },
        80,
        6,
    );
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let text_vh = 6usize.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
    let max_top = total_lines.saturating_sub(text_vh);
    assert!(
        state
            .views
            .get(&id)
            .expect("missing explorer view")
            .cursor
            .scroll_y_lines
            <= max_top
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn explorer_dash_parent_navigation_selects_previous_directory() {
    let root = std::env::temp_dir().join(format!(
        "redox_explorer_parent_dash_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    let child = root.join("child");
    fs::create_dir_all(root.join("aaa")).expect("failed to create fixture dir");
    fs::create_dir_all(root.join("zzz")).expect("failed to create fixture dir");
    fs::create_dir_all(&child).expect("failed to create fixture dir");
    let child_file = child.join("open.txt");
    fs::write(&child_file, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&child_file).expect("failed to open session");
    let mut state = EditorState::new(session);
    state
        .open_explorer_at_path(child.clone())
        .expect("failed to open explorer");

    state.apply_input(InputAction::SurfaceGoParent, 80, 24);

    let popup = state
        .explorer_popup()
        .expect("explorer popup should stay open");
    let expected_parent = fs::canonicalize(&root).expect("failed to canonicalize root");
    assert_eq!(popup.dir_path, expected_parent);

    let child_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "child/")
        .expect("child directory missing from parent listing");
    let active = state.session.active_id();
    let cursor_line = state
        .views
        .get(&active)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line;
    assert_eq!(cursor_line, child_line);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn explorer_parent_entry_navigation_selects_previous_directory() {
    let root = std::env::temp_dir().join(format!(
        "redox_explorer_parent_entry_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    let child = root.join("child");
    fs::create_dir_all(root.join("aaa")).expect("failed to create fixture dir");
    fs::create_dir_all(root.join("zzz")).expect("failed to create fixture dir");
    fs::create_dir_all(&child).expect("failed to create fixture dir");
    let child_file = child.join("open.txt");
    fs::write(&child_file, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&child_file).expect("failed to open session");
    let mut state = EditorState::new(session);
    state
        .open_explorer_at_path(child.clone())
        .expect("failed to open explorer");

    let parent_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "../")
        .expect("parent entry missing from listing");
    let active = state.session.active_id();
    state
        .views
        .get_mut(&active)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line = parent_line;

    state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

    let popup = state
        .explorer_popup()
        .expect("explorer popup should stay open");
    let expected_parent = fs::canonicalize(&root).expect("failed to canonicalize root");
    assert_eq!(popup.dir_path, expected_parent);

    let child_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "child/")
        .expect("child directory missing from parent listing");
    let cursor_line = state
        .views
        .get(&active)
        .expect("missing explorer view")
        .cursor
        .cursor
        .line;
    assert_eq!(cursor_line, child_line);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn viewport_down_and_up_center_cursor_in_normal_mode() {
    let path = temp_file_path("viewport_center_scroll");
    let text = large_text(300);
    let mut state = state_with_text(path.clone(), &text);
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(3, 0);

    let viewport_height_rows = 8usize;
    let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
    let center_row = text_vh / 2;

    state.apply_input(InputAction::ViewportDownCenter, 80, viewport_height_rows);
    let view = state.views.get(&id).expect("missing view");
    assert_eq!(view.cursor.scroll_y_lines, text_vh);
    assert_eq!(view.cursor.cursor.line, text_vh + center_row);

    state.apply_input(InputAction::ViewportUpCenter, 80, viewport_height_rows);
    let view = state.views.get(&id).expect("missing view");
    assert_eq!(view.cursor.scroll_y_lines, 0);
    assert_eq!(view.cursor.cursor.line, center_row);

    // A further Ctrl+U while already at top behaves like gg.
    state.apply_input(InputAction::ViewportUpCenter, 80, viewport_height_rows);
    let view = state.views.get(&id).expect("missing view");
    assert_eq!(view.cursor.scroll_y_lines, 0);
    assert_eq!(view.cursor.cursor.line, 0);

    let _ = fs::remove_file(path);
}

#[test]
fn viewport_down_reaches_last_line_after_repeated_presses() {
    let path = temp_file_path("viewport_down_reaches_eof");
    let text = large_text(120);
    let mut state = state_with_text(path.clone(), &text);
    let id = state.session.active_id();
    let viewport_height_rows = 8usize;
    let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
    let center_row = text_vh / 2;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let last_line = total_lines.saturating_sub(1);

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    for _ in 0..(total_lines / text_vh + 4) {
        state.apply_input(InputAction::ViewportDownCenter, 80, viewport_height_rows);
    }

    let view = state.views.get(&id).expect("missing view");
    assert_eq!(view.cursor.cursor.line, last_line);
    assert_eq!(
        view.cursor.scroll_y_lines,
        last_line.saturating_sub(center_row)
    );

    let _ = fs::remove_file(path);
}

#[test]
fn zz_centers_cursor_line_except_at_top_of_file() {
    let path = temp_file_path("zz_center_line");
    let text = large_text(120);
    let mut state = state_with_text(path.clone(), &text);
    let id = state.session.active_id();
    let viewport_height_rows = 8usize;
    let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
    let center_row = text_vh / 2;

    {
        let view = state.views.get_mut(&id).expect("missing view");
        view.cursor.cursor = Pos::new(0, 0);
        view.cursor.scroll_y_lines = 10;
    }
    state.apply_input(InputAction::CenterCursorLine, 80, viewport_height_rows);
    assert_eq!(
        state
            .views
            .get(&id)
            .expect("missing view")
            .cursor
            .scroll_y_lines,
        0
    );

    let last_line = state.session.active_buffer().len_lines().saturating_sub(1);
    {
        let view = state.views.get_mut(&id).expect("missing view");
        view.cursor.cursor = Pos::new(last_line, 0);
    }
    state.apply_input(InputAction::CenterCursorLine, 80, viewport_height_rows);
    let view = state.views.get(&id).expect("missing view");
    assert_eq!(
        view.cursor.scroll_y_lines,
        last_line.saturating_sub(center_row)
    );
    assert_eq!(view.cursor.cursor.line, last_line);

    let _ = fs::remove_file(path);
}

#[test]
fn open_line_below_enters_insert_and_inserts_blank_line() {
    let path = temp_file_path("open_line_below");
    let mut state = state_with_text(path.clone(), "one\ntwo");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::OpenLineBelow, 80, 24);

    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.session.active_buffer().to_string(), "one\n\ntwo");
    assert_eq!(
        state.views.get(&id).expect("missing view").cursor.cursor,
        Pos::new(1, 0)
    );

    let _ = fs::remove_file(path);
}

#[test]
fn open_line_above_enters_insert_and_inserts_blank_line() {
    let path = temp_file_path("open_line_above");
    let mut state = state_with_text(path.clone(), "one\ntwo");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);

    state.apply_input(InputAction::OpenLineAbove, 80, 24);

    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.session.active_buffer().to_string(), "one\n\ntwo");
    assert_eq!(
        state.views.get(&id).expect("missing view").cursor.cursor,
        Pos::new(1, 0)
    );

    let _ = fs::remove_file(path);
}

#[test]
fn first_edit_forces_full_load_before_mutation() {
    let path = temp_file_path("edit_force_full_load");
    let text = large_text(8000);
    let mut state = state_with_text(path.clone(), &text);
    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Loading
    );

    state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);
    state.apply_input(InputAction::InsertChar('X'), 80, 24);

    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Complete
    );
    assert_eq!(
        state.session.active_buffer().to_string(),
        format!("X{text}")
    );

    let _ = fs::remove_file(path);
}

#[test]
fn write_command_forces_full_load_while_loading() {
    let path = temp_file_path("write_force_full_load");
    let text = large_text(8500);
    let mut state = state_with_text(path.clone(), &text);
    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Loading
    );

    run_command(&mut state, "w");

    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Complete
    );
    assert_eq!(
        fs::read_to_string(&path).expect("failed to read file"),
        text
    );

    let _ = fs::remove_file(path);
}

#[test]
fn pump_active_loading_extends_loaded_content() {
    let path = temp_file_path("pump_loading_growth");
    let text = large_text(10_000);
    let mut state = state_with_text(path.clone(), &text);
    let before_lines = state.session.active_buffer().len_lines();

    for _ in 0..8 {
        state.pump_active_loading(20);
        state.apply_input(
            InputAction::Motion {
                motion: Motion::Down,
                count: 120,
            },
            80,
            24,
        );
    }

    let after_lines = state.session.active_buffer().len_lines();
    assert!(after_lines > before_lines);

    let _ = fs::remove_file(path);
}

#[test]
fn load_failure_sets_status_and_blocks_mutation() {
    let path = temp_file_path("load_failure_status");
    let mut file = fs::File::create(&path).expect("failed to create temp file");
    let prefix = "ok\n".repeat(30_000);
    file.write_all(prefix.as_bytes())
        .expect("failed to write prefix");
    file.write_all(&[0xff])
        .expect("failed to write invalid byte");
    file.flush().expect("failed to flush");

    let mut state =
        EditorState::new(EditorSession::open_initial_file(&path).expect("failed to open session"));
    let before = state.session.active_buffer().to_string();

    state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);
    state.apply_input(InputAction::InsertChar('x'), 80, 24);

    let msg = state.status_msg.as_deref().unwrap_or("");
    assert!(msg.contains("load failed"));
    assert_eq!(state.session.active_buffer().to_string(), before);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_mode_tracks_anchor_and_cursor_selection() {
    let path = temp_file_path("visual_char");
    let mut state = state_with_text(path.clone(), "abc\ndef\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    let (sel, mode) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert_eq!(mode, VisualModeKind::Char);
    assert_eq!(sel.anchor, Pos::new(0, 1));
    assert_eq!(sel.cursor, Pos::new(0, 1));

    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    let (sel, _) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert_eq!(sel.anchor, Pos::new(0, 1));
    assert_eq!(sel.cursor.line, 1);

    state.apply_input(InputAction::SetMode(InputMode::Normal), 80, 24);
    assert!(state.active_visual_selection().is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_mode_marks_line_selection_mode() {
    let path = temp_file_path("visual_line");
    let mut state = state_with_text(path.clone(), "a\nb\nc\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    let (sel, mode) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert_eq!(mode, VisualModeKind::Line);
    assert_eq!(sel.anchor.line, 1);
    assert_eq!(sel.cursor.line, 2);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_yank_private_copies_selection_and_exits_visual_mode() {
    let path = temp_file_path("visual_yank_private");
    let mut state = state_with_text(path.clone(), "alpha\nbeta\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 3,
        },
        80,
        24,
    );

    state.apply_input(InputAction::YankSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "alph");
    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(state.status_msg.as_deref(), Some("yanked"));
    assert_eq!(
        state.one_shot_highlight(),
        Some((
            Selection::new(Pos::new(0, 0), Pos::new(0, 3)),
            VisualModeKind::Char
        ))
    );
    assert!(state.take_pending_system_clipboard().is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_leader_y_queues_system_clipboard_text() {
    let path = temp_file_path("visual_yank_system");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    state.apply_input(InputAction::YankSelectionSystem, 80, 24);

    assert_eq!(state.private_register, "one\ntwo\n");
    assert_eq!(
        state.take_pending_system_clipboard().as_deref(),
        Some("one\ntwo\n")
    );
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_yank_private_without_motion_includes_cursor_char() {
    let path = temp_file_path("visual_yank_single_char");
    let mut state = state_with_text(path.clone(), "alpha\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 2);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(InputAction::YankSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "p");
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_mode_marks_block_selection_mode() {
    let path = temp_file_path("visual_block");
    let mut state = state_with_text(path.clone(), "abcd\nefgh\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );

    let (sel, mode) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert_eq!(mode, VisualModeKind::Block);
    assert_eq!(sel.anchor, Pos::new(0, 1));
    assert_eq!(sel.cursor, Pos::new(1, 2));

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_yank_private_copies_rectangular_selection() {
    let path = temp_file_path("visual_block_yank_private");
    let mut state = state_with_text(path.clone(), "abcd\nefgh\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::YankSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "bc\nfg");
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_delete_private_cuts_charwise_selection() {
    let path = temp_file_path("visual_delete_private");
    let mut state = state_with_text(path.clone(), "alpha\nbeta\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 2,
        },
        80,
        24,
    );

    state.apply_input(InputAction::DeleteSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "alp");
    assert_eq!(state.session.active_buffer().to_string(), "ha\nbeta\n");
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_change_private_cuts_charwise_selection_and_enters_insert() {
    let path = temp_file_path("visual_change_private");
    let mut state = state_with_text(path.clone(), "alpha\nbeta\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 2,
        },
        80,
        24,
    );

    state.apply_input(InputAction::ChangeSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "alp");
    assert_eq!(state.session.active_buffer().to_string(), "ha\nbeta\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_delete_private_cuts_full_lines() {
    let path = temp_file_path("visual_line_delete_private");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    state.apply_input(InputAction::DeleteSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "one\ntwo\n");
    assert_eq!(state.session.active_buffer().to_string(), "three\n");
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_change_private_cuts_full_lines_and_enters_insert() {
    let path = temp_file_path("visual_line_change_private");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    state.apply_input(InputAction::ChangeSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "one\ntwo\n");
    assert_eq!(state.session.active_buffer().to_string(), "\nthree\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_delete_inner_word_cuts_current_word() {
    let path = temp_file_path("text_object_delete_inner_word");
    let mut state = state_with_text(path.clone(), "alpha beta gamma\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 7);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Delete,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "beta");
    assert_eq!(state.session.active_buffer().to_string(), "alpha  gamma\n");
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_change_around_paragraph_leaves_single_blank_line() {
    let path = temp_file_path("text_object_change_around_paragraph");
    let mut state = state_with_text(path.clone(), "one\ntwo\n\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Change,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Paragraph,
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "one\ntwo\n\n");
    assert_eq!(state.session.active_buffer().to_string(), "\nthree\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_yank_sets_flash_highlight() {
    let path = temp_file_path("text_object_yank_inner_word");
    let mut state = state_with_text(path.clone(), "alpha beta gamma\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 7);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Yank,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "beta");
    assert_eq!(
        state.one_shot_highlight(),
        Some((
            Selection::new(Pos::new(0, 6), Pos::new(0, 9)),
            VisualModeKind::Char
        ))
    );

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_change_inner_brackets_preserves_delimiters() {
    let path = temp_file_path("text_object_change_inner_brackets");
    let mut state = state_with_text(path.clone(), "foo[bar[baz]]\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 9);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Change,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "baz");
    assert_eq!(state.session.active_buffer().to_string(), "foo[bar[]]\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 8));

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_change_inner_parentheses_can_jump_to_same_line_pair() {
    let path = temp_file_path("text_object_change_inner_parentheses_same_line");
    let mut state = state_with_text(path.clone(), "let value = foo(bar);\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 4);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Change,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::Parentheses),
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "bar");
    assert_eq!(
        state.session.active_buffer().to_string(),
        "let value = foo();\n"
    );
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 16));

    let _ = fs::remove_file(path);
}

#[test]
fn text_object_change_inner_double_quotes_can_jump_to_same_line_pair() {
    let path = temp_file_path("text_object_change_inner_double_quotes_same_line");
    let mut state = state_with_text(path.clone(), "let value = \"bar\";\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Change,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes),
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(state.private_register, "bar");
    assert_eq!(
        state.session.active_buffer().to_string(),
        "let value = \"\";\n"
    );
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 13));

    let _ = fs::remove_file(path);
}

#[test]
fn visual_text_object_inner_word_replaces_selection_and_jumps_to_word() {
    let path = temp_file_path("visual_text_object_inner_word");
    let mut state = state_with_text(path.clone(), "alpha  beta\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 5);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Select,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            }),
        },
        80,
        24,
    );

    let (selection, mode) = state.active_visual_selection().expect("visual selection");
    assert_eq!(mode, VisualModeKind::Char);
    assert_eq!(selection.anchor, Pos::new(0, 7));
    assert_eq!(selection.cursor, Pos::new(0, 10));
    assert_eq!(
        state
            .session
            .active_buffer()
            .visual_selection_text(selection, mode),
        "beta"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_text_object_inner_big_word_selects_non_whitespace_run() {
    let path = temp_file_path("visual_text_object_inner_big_word");
    let mut state = state_with_text(
        path.clone(),
        "byte_idx.saturating_add(grapheme.len()); next\n",
    );
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 22);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Select,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::BigWord,
                count: 1,
            }),
        },
        80,
        24,
    );

    let (selection, mode) = state.active_visual_selection().expect("visual selection");
    assert_eq!(mode, VisualModeKind::Char);
    assert_eq!(
        state
            .session
            .active_buffer()
            .visual_selection_text(selection, mode),
        "byte_idx.saturating_add(grapheme.len());"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_text_object_inner_paragraph_switches_to_linewise_selection() {
    let path = temp_file_path("visual_text_object_inner_paragraph");
    let mut state = state_with_text(path.clone(), "zero\n\none\ntwo\n\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(2, 1);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Select,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Paragraph,
                count: 1,
            }),
        },
        80,
        24,
    );

    let (selection, mode) = state.active_visual_selection().expect("visual selection");
    assert_eq!(mode, VisualModeKind::Line);
    assert_eq!(selection.anchor, Pos::new(2, 0));
    assert_eq!(selection.cursor, Pos::new(3, 0));
    assert_eq!(
        state
            .session
            .active_buffer()
            .visual_selection_text(selection, mode),
        "one\ntwo\n"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_text_object_around_brackets_uses_same_span_as_operator() {
    let path = temp_file_path("visual_text_object_around_brackets");
    let mut state = state_with_text(path.clone(), "x [ab] y\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 7);

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Select,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                count: 1,
            }),
        },
        80,
        24,
    );

    let (selection, mode) = state.active_visual_selection().expect("visual selection");
    assert_eq!(mode, VisualModeKind::Char);
    assert_eq!(
        state
            .session
            .active_buffer()
            .visual_selection_text(selection, mode),
        "[ab]"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_text_object_selection_forces_full_load_before_resolving() {
    let path = temp_file_path("visual_text_object_force_full_load");
    let text = large_text(8500);
    let mut state = state_with_text(path.clone(), &text);
    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Loading
    );

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::OperateTarget {
            operator: TextObjectOperator::Select,
            target: OperatorTarget::TextObject(TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Paragraph,
                count: 1,
            }),
        },
        80,
        24,
    );

    assert_eq!(
        state.session.active_buffer_load_status().phase,
        BufferLoadPhase::Complete
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_line_delete_private_cuts_all_selected_lines() {
    let path = temp_file_path("visual_line_delete_three_lines");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\nfour\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 2,
        },
        80,
        24,
    );
    state.apply_input(InputAction::DeleteSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "one\ntwo\nthree\n");
    assert_eq!(state.session.active_buffer().to_string(), "four\n");
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_delete_private_cuts_rectangular_selection() {
    let path = temp_file_path("visual_block_delete_private");
    let mut state = state_with_text(path.clone(), "abcd\nefgh\nijkl\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 2,
        },
        80,
        24,
    );
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::DeleteSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "bc\nfg\njk");
    assert_eq!(state.session.active_buffer().to_string(), "ad\neh\nil\n");
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_change_private_cuts_rectangular_selection_and_enters_insert() {
    let path = temp_file_path("visual_block_change_private");
    let mut state = state_with_text(path.clone(), "abcd\nefgh\nijkl\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 2,
        },
        80,
        24,
    );
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        80,
        24,
    );
    state.apply_input(InputAction::ChangeSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "bc\nfg\njk");
    assert_eq!(state.session.active_buffer().to_string(), "ad\neh\nil\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_delete_private_removes_fully_covered_lines() {
    let path = temp_file_path("visual_block_delete_full_lines");
    let mut state = state_with_text(path.clone(), "{\nalpha\nbeta\n}\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualBlock), 80, 24);
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(2, 8);
    state.apply_input(InputAction::DeleteSelectionPrivate, 80, 24);

    assert_eq!(state.private_register, "alpha\nbeta");
    assert_eq!(state.session.active_buffer().to_string(), "{\n}\n");
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn normal_x_deletes_char_without_modifying_private_register() {
    let path = temp_file_path("normal_x_no_yank");
    let mut state = state_with_text(path.clone(), "alpha\n");
    state.private_register = "keep".to_string();

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::DeleteCharNoYank, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "apha\n");
    assert_eq!(state.private_register, "keep");
    assert!(state.status_msg.is_none());
    let _ = fs::remove_file(path);
}

#[test]
fn visual_x_deletes_selection_without_modifying_private_register() {
    let path = temp_file_path("visual_x_no_yank");
    let mut state = state_with_text(path.clone(), "alpha\nbeta\n");
    state.private_register = "keep".to_string();
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 2,
        },
        80,
        24,
    );

    state.apply_input(InputAction::DeleteSelectionNoYank, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "ha\nbeta\n");
    assert_eq!(state.private_register, "keep");
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.status_msg.is_none());
    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_shift_p_pastes_private_register_before_cursor() {
    let path = temp_file_path("paste_before_charwise");
    let mut state = state_with_text(path.clone(), "abc\n");
    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(InputAction::YankSelectionPrivate, 80, 24);

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 2);
    state.apply_input(InputAction::PastePrivateRegisterBefore, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "abac\n");
    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_dd_cuts_current_line() {
    let path = temp_file_path("dd_cut_line");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 1);

    state.apply_input(InputAction::DeleteCurrentLinePrivate { count: 1 }, 80, 24);

    assert_eq!(state.private_register, "two\n");
    assert_eq!(state.session.active_buffer().to_string(), "one\nthree\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 0));
    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_yy_yanks_current_line_and_sets_flash() {
    let path = temp_file_path("yy_yank_line");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 1);

    state.apply_input(InputAction::YankCurrentLinePrivate { count: 1 }, 80, 24);

    assert_eq!(state.private_register, "two\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    assert_eq!(state.status_msg.as_deref(), Some("yanked line"));
    assert_eq!(
        state.one_shot_highlight(),
        Some((
            Selection::new(Pos::new(1, 0), Pos::new(1, 0)),
            VisualModeKind::Line
        ))
    );
    let _ = fs::remove_file(path);
}

#[test]
fn yank_flash_persists_for_two_frames() {
    let path = temp_file_path("yy_yank_flash_duration");
    let mut state = state_with_text(path.clone(), "one\ntwo\n");

    state.apply_input(InputAction::YankCurrentLinePrivate { count: 1 }, 80, 24);
    assert!(state.one_shot_highlight().is_some());

    state.advance_one_shot_highlight();
    assert!(state.one_shot_highlight().is_some());

    state.advance_one_shot_highlight();
    assert!(state.one_shot_highlight().is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn one_shot_highlight_is_scoped_to_its_buffer() {
    let path_a = temp_file_path("one_shot_highlight_buffer_a");
    let path_b = temp_file_path("one_shot_highlight_buffer_b");
    let mut state = state_with_text(path_a.clone(), "one\ntwo\n");
    fs::write(&path_b, "alpha\nbeta\n").expect("failed to write test file");

    let id_a = state.session.active_id();
    state.apply_input(InputAction::YankCurrentLinePrivate { count: 1 }, 80, 24);
    let expected = Some((
        Selection::new(Pos::new(0, 0), Pos::new(0, 0)),
        VisualModeKind::Line,
    ));
    assert_eq!(state.one_shot_highlight(), expected);

    run_command(&mut state, &format!("e {}", path_b.display()));
    let id_b = state.session.active_id();
    assert_ne!(id_a, id_b);
    assert_eq!(state.one_shot_highlight(), None);

    state.advance_one_shot_highlight();
    run_command(&mut state, "bp");
    assert_eq!(state.session.active_id(), id_a);
    assert_eq!(state.one_shot_highlight(), expected);

    state.advance_one_shot_highlight();
    assert!(state.one_shot_highlight().is_some());

    state.advance_one_shot_highlight();
    assert!(state.one_shot_highlight().is_none());

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn repeat_search_does_not_reuse_active_match_positions_across_buffers() {
    let path_a = temp_file_path("search_state_buffer_a");
    let path_b = temp_file_path("search_state_buffer_b");
    let mut state = state_with_text(path_a.clone(), "alpha beta gamma beta\n");
    fs::write(&path_b, "beta gamma beta\n").expect("failed to write test file");

    let id_a = state.session.active_id();
    state
        .views
        .get_mut(&id_a)
        .expect("missing view for buffer A")
        .cursor
        .cursor = Pos::new(0, 7);

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "beta".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 17));

    run_command(&mut state, &format!("e {}", path_b.display()));
    let id_b = state.session.active_id();
    assert_ne!(id_a, id_b);
    state
        .views
        .get_mut(&id_b)
        .expect("missing view for buffer B")
        .cursor
        .cursor = Pos::new(0, 1);

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 11));

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn normal_mode_cc_changes_current_line_and_enters_insert() {
    let path = temp_file_path("cc_change_line");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 2);

    state.apply_input(InputAction::ChangeCurrentLinePrivate { count: 1 }, 80, 24);

    assert_eq!(state.private_register, "two\n");
    assert_eq!(state.session.active_buffer().to_string(), "one\n\nthree\n");
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 0));
    assert!(state.status_msg.is_none());
    let _ = fs::remove_file(path);
}

#[test]
fn visual_shift_j_and_k_move_selected_lines() {
    let path = temp_file_path("visual_move_lines");
    let mut state = state_with_text(path.clone(), "one\ntwo\nthree\nfour\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    state.apply_input(InputAction::MoveVisualSelectionDown { count: 1 }, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "one\nfour\ntwo\nthree\n"
    );
    state.apply_input(InputAction::MoveVisualSelectionUp { count: 1 }, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "one\ntwo\nthree\nfour\n"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn visual_move_last_line_up_does_not_join_at_eof() {
    let path = temp_file_path("visual_move_last_line_up_eof");
    let mut state = state_with_text(path.clone(), "one\ntwo");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(InputAction::MoveVisualSelectionUp { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "two\none");

    let _ = fs::remove_file(path);
}

#[test]
fn visual_move_second_last_line_down_does_not_join_at_eof() {
    let path = temp_file_path("visual_move_second_last_down_eof");
    let mut state = state_with_text(path.clone(), "one\ntwo");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(InputAction::MoveVisualSelectionDown { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "two\none");

    let _ = fs::remove_file(path);
}

#[test]
fn visual_move_into_newline_only_last_line_does_not_panic_or_join() {
    let path = temp_file_path("visual_move_into_blank_last_line");
    let mut state = state_with_text(path.clone(), "one\ntwo\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 0);

    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(InputAction::MoveVisualSelectionDown { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "one\n\ntwo");

    let _ = fs::remove_file(path);
}

#[test]
fn visual_tab_and_shift_tab_indent_and_outdent_selection() {
    let path = temp_file_path("visual_indent_outdent");
    let mut state = state_with_text(path.clone(), "one\ntwo\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        80,
        24,
    );

    state.apply_input(InputAction::IndentVisualSelection { count: 1 }, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "\tone\n\ttwo\n");

    state.apply_input(InputAction::OutdentVisualSelection { count: 1 }, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "one\ntwo\n");

    let _ = fs::remove_file(path);
}
