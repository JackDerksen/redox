use super::*;
use redox_core::{motion::Motion, BufferLoadPhase};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::input::{InputAction, InputMode, InsertKind};
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
    assert!(state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .any(|line| line == ".."));

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
        *buffer = TextBuffer::from_str("..\nrenamed.txt\ncreated.txt");
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

    assert!(state.about_popup().is_some());
    assert!(state.active_display_name().contains("[about]"));

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
    let (sel, line_mode) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert!(!line_mode);
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

    let (sel, line_mode) = state
        .active_visual_selection()
        .expect("visual selection should exist");
    assert!(line_mode);
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
    assert_eq!(state.status_msg.as_deref(), Some("deleted"));

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
    assert_eq!(state.status_msg.as_deref(), Some("deleted"));

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
    assert_eq!(state.status_msg.as_deref(), Some("deleted"));
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
    assert_eq!(state.status_msg.as_deref(), Some("deleted"));
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
