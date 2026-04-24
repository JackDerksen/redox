use super::*;
use redox_core::{
    BufferLoadPhase, DelimiterKind, TextObjectKind, TextObjectScope, TextObjectSpec,
    VisualModeKind, motion::Motion,
};
use std::fs;
use std::io::Write;
use std::panic::{self, UnwindSafe};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::input::{InputAction, InputMode, InsertKind, OperatorTarget, TextObjectOperator};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::syntax::SyntaxLanguage;

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

fn enter_command_mode(state: &mut EditorState) {
    state.apply_input(InputAction::EnterCommand, 80, 24);
}

fn wait_for_rust_syntax_cache(state: &mut EditorState, id: redox_core::BufferId) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        state.poll_analysis_results();
        if state
            .views
            .get(&id)
            .is_some_and(|view| view.syntax_highlighter.has_cache_for(SyntaxLanguage::Rust))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let view_exists = state.views.contains_key(&id);
    let has_rust_cache = state
        .views
        .get(&id)
        .is_some_and(|view| view.syntax_highlighter.has_cache_for(SyntaxLanguage::Rust));
    panic!(
        "rust syntax cache was not populated before deadline; view_exists={view_exists}, has_rust_cache={has_rust_cache}"
    );
}

fn wait_for_finder_popup(
    state: &mut EditorState,
    predicate: impl Fn(&FinderPopup) -> bool,
) -> FinderPopup {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        state.poll_finder_results();
        if let Some(popup) = state.finder_popup()
            && predicate(&popup)
        {
            return popup;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let popup = state.finder_popup();
    panic!("finder popup did not reach expected state before deadline; popup={popup:?}");
}

fn wait_for_finder_index_idle(state: &mut EditorState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        state.poll_finder_results();
        if state.finder_index_worker.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    panic!("finder index worker did not finish before deadline");
}

fn expire_status_after_timeout(state: &mut EditorState) {
    state.expire_status_message(Instant::now() + Duration::from_secs(10));
}

fn lock_global_test_state() -> std::sync::MutexGuard<'static, ()> {
    global_test_state_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_global_test_state_lock<T>(f: impl FnOnce() -> T) -> T {
    let _lock = lock_global_test_state();
    f()
}

fn with_isolated_launch_env<T>(tag: &str, f: impl FnOnce(PathBuf) -> T + UnwindSafe) -> T {
    let _lock = lock_global_test_state();
    let root = temp_file_path(tag);
    fs::create_dir_all(&root).expect("failed to create temp root");
    let previous_cwd = std::env::current_dir().expect("failed to capture cwd");
    let previous_xdg = std::env::var_os("XDG_CONFIG_HOME");
    let config_root = root.join("config");

    std::env::set_current_dir(&root).expect("failed to switch cwd");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config_root);
    }

    let result = panic::catch_unwind(|| f(root.clone()));

    std::env::set_current_dir(&previous_cwd).expect("failed to restore cwd");
    match previous_xdg {
        Some(value) => unsafe {
            std::env::set_var("XDG_CONFIG_HOME", value);
        },
        None => unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        },
    }
    let _ = fs::remove_dir_all(&root);

    match result {
        Ok(value) => value,
        Err(err) => panic::resume_unwind(err),
    }
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
fn quick_pin_current_file_opens_selector_before_persisting() {
    with_isolated_launch_env("quick_pin_current_file_selector", |root| {
        let file_path = root.join("alpha.txt");
        fs::write(&file_path, "alpha\n").expect("failed to write file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let mut state = EditorState::new(session);
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        let popup = state.pin_selector_popup().expect("pin selector popup");
        assert_eq!(popup.path_label, "alpha.txt");
        assert_eq!(popup.selected, 0);

        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let canonical = fs::canonicalize(&file_path).expect("canonical path");
        assert_eq!(
            state.pinned_files_for_test(),
            std::slice::from_ref(&canonical)
        );

        let config_path = root.join("config").join("redox").join("pinned_files.txt");
        let saved = fs::read_to_string(config_path).expect("failed to read pin file");
        assert!(saved.contains(&canonical.display().to_string()));
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn pinned_files_save_as_json_array_atomically() {
    with_isolated_launch_env("pinned_files_save_json", |root| {
        let file_path = root.join("line\nbreak.txt");
        fs::write(&file_path, "alpha\n").expect("failed to write file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let mut state = EditorState::new(session);
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::AssignPinSlot { slot: 2 }, 80, 24);

        let canonical = fs::canonicalize(&file_path).expect("canonical path");
        let config_file = root.join("config").join("redox").join("pinned_files.txt");
        let saved = fs::read_to_string(&config_file).expect("failed to read pin file");
        let slots: Vec<Option<String>> =
            serde_json::from_str(&saved).expect("pin file should be JSON");
        assert_eq!(slots.len(), 5);
        assert_eq!(slots[0], None);
        assert_eq!(slots[1], None);
        assert_eq!(slots[2], Some(canonical.display().to_string()));
        assert!(!config_file.with_extension("tmp").exists());

        let session = EditorSession::open_initial_file(&file_path).expect("failed to reopen file");
        let reloaded = EditorState::new(session);
        assert_eq!(
            reloaded.pin_slots_for_test(),
            vec![None, None, Some(canonical), None, None]
        );
    });
}

#[test]
fn pinned_files_load_legacy_format_without_trimming_paths() {
    with_isolated_launch_env("pinned_files_legacy_trailing_space", |root| {
        let file_path = root.join("trail   ");
        fs::write(&file_path, "alpha\n").expect("failed to write file");
        let canonical = fs::canonicalize(&file_path).expect("canonical path");

        let config_file = root.join("config").join("redox").join("pinned_files.txt");
        fs::create_dir_all(config_file.parent().expect("pin config dir"))
            .expect("failed to create config dir");
        fs::write(&config_file, format!("-\n{}\n", file_path.display()))
            .expect("failed to write legacy pin file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let state = EditorState::new(session);

        assert_eq!(
            state.pin_slots_for_test(),
            vec![None, Some(canonical), None, None, None]
        );
    });
}

#[test]
fn pinned_files_load_json_before_legacy_format() {
    with_isolated_launch_env("pinned_files_load_json", |root| {
        let first_path = root.join("first.txt");
        let extra_path = root.join("extra.txt");
        fs::write(&first_path, "first\n").expect("failed to write first file");
        fs::write(&extra_path, "extra\n").expect("failed to write extra file");
        let canonical = fs::canonicalize(&first_path).expect("canonical path");

        let config_file = root.join("config").join("redox").join("pinned_files.txt");
        fs::create_dir_all(config_file.parent().expect("pin config dir"))
            .expect("failed to create config dir");
        let saved = serde_json::json!([
            null,
            first_path.display().to_string(),
            "-",
            "relative.txt",
            first_path.display().to_string(),
            extra_path.display().to_string()
        ]);
        fs::write(&config_file, saved.to_string()).expect("failed to write JSON pin file");

        let session =
            EditorSession::open_initial_file(&first_path).expect("failed to open session");
        let state = EditorState::new(session);

        assert_eq!(
            state.pin_slots_for_test(),
            vec![None, Some(canonical), None, None, None]
        );
    });
}

#[test]
fn finder_does_not_open_while_about_popup_is_active() {
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state.command_open_about();

        assert!(state.about_popup().is_some());

        state.apply_input(InputAction::OpenFinder, 80, 24);

        assert!(state.about_popup().is_some());
        assert!(state.finder_popup().is_none());
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn finder_does_not_open_while_explorer_popup_is_active() {
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state
            .open_explorer_at_path(state.session.launch_dir().to_path_buf())
            .expect("failed to open explorer");

        assert!(state.explorer_popup().is_some());

        state.apply_input(InputAction::OpenFinder, 80, 24);

        assert!(state.explorer_popup().is_some());
        assert!(state.finder_popup().is_none());
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn quick_pin_does_not_open_pinboard_while_about_popup_is_active() {
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state.command_open_about();

        assert!(state.about_popup().is_some());

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

        assert!(state.about_popup().is_some());
        assert!(state.pin_selector_popup().is_none());
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn quick_pin_does_not_open_pinboard_while_perf_popup_is_active() {
    let path = temp_file_path("quick_pin_blocked_by_perf");
    let mut state = state_with_text(path.clone(), "alpha\n");
    state.command_toggle_perf();

    assert!(state.perf_popup().is_some());

    state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

    assert!(state.perf_popup().is_some());
    assert!(state.pin_selector_popup().is_none());
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn open_pinned_slot_from_normal_mode_opens_file_without_changing_launch_dir() {
    with_isolated_launch_env("open_pinned_slot_normal_mode", |root| {
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        fs::write(&first_path, "first\n").expect("failed to write first file");
        fs::write(&second_path, "second\n").expect("failed to write second file");

        let session =
            EditorSession::open_initial_file(&first_path).expect("failed to open session");
        let mut state = EditorState::new(session);
        let launch_dir = state.session.launch_dir().to_path_buf();

        let _ = state
            .session
            .open_file(&second_path)
            .expect("failed to switch file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let _ = state
            .session
            .open_file(&first_path)
            .expect("failed to return to first file");
        state.apply_input(InputAction::OpenPinnedSlot { slot: 0 }, 80, 24);

        let second_canonical = fs::canonicalize(&second_path).expect("second canonical");
        assert_eq!(
            state.session.active_meta().path.as_deref(),
            Some(second_canonical.as_path())
        );
        assert_eq!(state.session.launch_dir(), launch_dir.as_path());
    });
}

#[test]
fn open_pinned_slot_from_visual_mode_clears_visual_state() {
    with_isolated_launch_env("open_pinned_slot_visual_mode", |root| {
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        fs::write(&first_path, "first\n").expect("failed to write first file");
        fs::write(&second_path, "second\n").expect("failed to write second file");

        let session =
            EditorSession::open_initial_file(&first_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        let _ = state
            .session
            .open_file(&second_path)
            .expect("failed to switch to second file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let _ = state
            .session
            .open_file(&first_path)
            .expect("failed to return to first file");
        state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
        assert_eq!(state.mode, EditorMode::Visual);
        assert!(state.active_visual_selection().is_some());

        state.apply_input(InputAction::OpenPinnedSlot { slot: 0 }, 80, 24);

        let second_canonical = fs::canonicalize(&second_path).expect("second canonical");
        assert_eq!(state.mode, EditorMode::Normal);
        assert!(state.active_visual_selection().is_none());
        assert_eq!(
            state.session.active_meta().path.as_deref(),
            Some(second_canonical.as_path())
        );
    });
}

#[test]
fn failed_pin_assignment_rolls_back_and_keeps_selector_open() {
    with_isolated_launch_env("pin_assignment_save_failure", |root| {
        let config_blocker = root.join("config");
        fs::write(&config_blocker, "not a directory\n").expect("failed to write config blocker");
        let file_path = root.join("alpha.txt");
        fs::write(&file_path, "alpha\n").expect("failed to write file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        assert!(state.pin_selector_popup().is_some());
        assert!(state.pinned_files_for_test().is_empty());
        assert!(
            state
                .status_msg
                .as_deref()
                .is_some_and(|msg| msg.starts_with("pin save failed:"))
        );
    });
}

#[test]
fn failed_pin_reorder_rolls_back_in_memory_slots() {
    with_isolated_launch_env("pin_reorder_save_failure", |root| {
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        fs::write(&first_path, "first\n").expect("failed to write first file");
        fs::write(&second_path, "second\n").expect("failed to write second file");

        let session =
            EditorSession::open_initial_file(&first_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);
        let _ = state
            .session
            .open_file(&second_path)
            .expect("failed to switch to second file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let original_slots = state.pin_slots_for_test();
        let config_file = root.join("config").join("redox").join("pinned_files.txt");
        fs::remove_file(&config_file).expect("failed to remove pin file");
        fs::remove_dir(config_file.parent().expect("pin config dir"))
            .expect("failed to remove pin config dir");
        fs::write(
            config_file.parent().expect("pin config dir"),
            "not a directory\n",
        )
        .expect("failed to write config blocker");

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorReorderUp, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        assert_eq!(state.pin_slots_for_test(), original_slots);
        assert!(
            state
                .status_msg
                .as_deref()
                .is_some_and(|msg| msg.starts_with("pin save failed:"))
        );
    });
}

#[test]
fn pin_selector_shift_enter_assigns_current_file_to_selected_slot() {
    with_isolated_launch_env("pin_selector_shift_enter_assigns_current_file", |root| {
        let file_path = root.join("alpha.txt");
        fs::write(&file_path, "alpha\n").expect("failed to write file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        let popup = state.pin_selector_popup().expect("pin selector popup");
        assert_eq!(popup.selected, 0);

        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let canonical = fs::canonicalize(&file_path).expect("canonical path");
        assert_eq!(state.pinned_files_for_test(), &[canonical]);
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn pin_selector_enter_opens_selected_pinned_file() {
    with_isolated_launch_env("pin_selector_enter_opens_selected_pinned_file", |root| {
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        fs::write(&first_path, "first\n").expect("failed to write first file");
        fs::write(&second_path, "second\n").expect("failed to write second file");

        let session =
            EditorSession::open_initial_file(&first_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let _ = state
            .session
            .open_file(&second_path)
            .expect("failed to switch to second file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::AssignPinSlot { slot: 1 }, 80, 24);

        let _ = state
            .session
            .open_file(&second_path)
            .expect("failed to return to second file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorMovePrev, 80, 24);
        state.apply_input(InputAction::PinSelectorOpenSelected, 80, 24);

        let first_canonical = fs::canonicalize(&first_path).expect("first canonical");
        assert_eq!(
            state.session.active_meta().path.as_deref(),
            Some(first_canonical.as_path())
        );
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn explorer_stays_in_previous_context_after_opening_pinned_file() {
    with_isolated_launch_env(
        "explorer_stays_in_previous_context_after_pinned_open",
        |root| {
            let left_dir = root.join("left");
            let right_dir = root.join("right");
            fs::create_dir_all(&left_dir).expect("failed to create left dir");
            fs::create_dir_all(&right_dir).expect("failed to create right dir");

            let left_file = left_dir.join("left.txt");
            let pinned_file = right_dir.join("pinned.txt");
            fs::write(&left_file, "left\n").expect("failed to write left file");
            fs::write(&pinned_file, "pinned\n").expect("failed to write pinned file");

            let session =
                EditorSession::open_initial_file(&left_file).expect("failed to open session");
            let mut state = EditorState::new(session);

            let _ = state
                .session
                .open_file(&pinned_file)
                .expect("failed to switch to pinned file");
            state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
            state.apply_input(InputAction::PinSelectorAssign, 80, 24);

            let _ = state
                .session
                .open_file(&left_file)
                .expect("failed to return to left file");
            state.apply_input(InputAction::OpenPinnedSlot { slot: 0 }, 80, 24);

            let pinned_canonical = fs::canonicalize(&pinned_file).expect("pinned canonical");
            assert_eq!(
                state.session.active_meta().path.as_deref(),
                Some(pinned_canonical.as_path())
            );

            state.apply_input(InputAction::OpenExplorer, 80, 24);

            let popup = state.explorer_popup().expect("explorer popup");
            assert_eq!(
                popup.dir_path,
                fs::canonicalize(&left_dir).expect("left dir canonical")
            );
        },
    );
}

#[test]
fn explorer_stays_in_previous_context_after_opening_pinned_finder_entry() {
    with_isolated_launch_env(
        "explorer_stays_in_previous_context_after_pinned_finder_entry",
        |root| {
            let left_dir = root.join("left");
            let right_dir = root.join("right");
            fs::create_dir_all(&left_dir).expect("failed to create left dir");
            fs::create_dir_all(&right_dir).expect("failed to create right dir");

            let left_file = left_dir.join("left.txt");
            let pinned_file = right_dir.join("pinned.txt");
            fs::write(&left_file, "left\n").expect("failed to write left file");
            fs::write(&pinned_file, "pinned\n").expect("failed to write pinned file");

            let session =
                EditorSession::open_initial_file(&left_file).expect("failed to open session");
            let mut state = EditorState::new(session);

            let _ = state
                .session
                .open_file(&pinned_file)
                .expect("failed to switch to pinned file");
            state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
            state.apply_input(InputAction::PinSelectorAssign, 80, 24);

            let _ = state
                .session
                .open_file(&left_file)
                .expect("failed to return to left file");
            state.apply_input(InputAction::OpenFinder, 80, 24);

            let finder = state.finder_popup().expect("finder popup");
            assert!(finder.entries[0].is_pinned);

            for _ in 0..finder.entries.len() {
                state.apply_input(InputAction::FinderMovePrev, 80, 24);
            }
            state.apply_input(InputAction::FinderEnter, 80, 24);

            let pinned_canonical = fs::canonicalize(&pinned_file).expect("pinned canonical");
            assert_eq!(
                state.session.active_meta().path.as_deref(),
                Some(pinned_canonical.as_path())
            );

            state.apply_input(InputAction::OpenExplorer, 80, 24);

            let popup = state.explorer_popup().expect("explorer popup");
            assert_eq!(
                popup.dir_path,
                fs::canonicalize(&left_dir).expect("left dir canonical")
            );
        },
    );
}

#[test]
fn explorer_opened_from_pinned_file_keeps_pinned_file_as_background() {
    with_isolated_launch_env("explorer_background_stays_on_pinned_file", |root| {
        let left_dir = root.join("left");
        let right_dir = root.join("right");
        fs::create_dir_all(&left_dir).expect("failed to create left dir");
        fs::create_dir_all(&right_dir).expect("failed to create right dir");

        let left_file = left_dir.join("left.txt");
        let pinned_file = right_dir.join("pinned.txt");
        fs::write(&left_file, "left\n").expect("failed to write left file");
        fs::write(&pinned_file, "pinned\n").expect("failed to write pinned file");

        let session = EditorSession::open_initial_file(&left_file).expect("failed to open session");
        let mut state = EditorState::new(session);

        let _ = state
            .session
            .open_file(&pinned_file)
            .expect("failed to switch to pinned file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        let _ = state
            .session
            .open_file(&left_file)
            .expect("failed to return to left file");
        state.apply_input(InputAction::OpenPinnedSlot { slot: 0 }, 80, 24);
        let pinned_id = state.session.active_id();

        state.apply_input(InputAction::OpenExplorer, 80, 24);

        assert!(state.explorer_popup().is_some());
        assert_eq!(state.explorer_background_buffer_id(), Some(pinned_id));
    });
}

#[test]
fn quick_pin_enters_selector_when_pin_list_is_full() {
    with_isolated_launch_env("quick_pin_full_selector", |root| {
        let files = (0..6)
            .map(|idx| {
                let path = root.join(format!("file-{idx}.txt"));
                fs::write(&path, format!("file-{idx}\n")).expect("failed to write file");
                path
            })
            .collect::<Vec<_>>();

        let session = EditorSession::open_initial_file(&files[0]).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);
        for path in &files[1..5] {
            let _ = state
                .session
                .open_file(path)
                .expect("failed to switch file");
            state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
            state.apply_input(InputAction::PinSelectorAssign, 80, 24);
        }

        let _ = state
            .session
            .open_file(&files[5])
            .expect("failed to switch file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        assert!(state.pin_selector_popup().is_some());

        state.apply_input(InputAction::AssignPinSlot { slot: 1 }, 80, 24);

        let expected = vec![
            fs::canonicalize(&files[0]).expect("pin 0"),
            fs::canonicalize(&files[5]).expect("pin 5"),
            fs::canonicalize(&files[2]).expect("pin 2"),
            fs::canonicalize(&files[3]).expect("pin 3"),
            fs::canonicalize(&files[4]).expect("pin 4"),
        ];
        assert_eq!(state.pinned_files_for_test(), expected.as_slice());
        assert_eq!(state.mode, EditorMode::Normal);
    });
}

#[test]
fn pin_selector_can_assign_directly_to_empty_later_slot() {
    with_isolated_launch_env("pin_selector_assign_empty_later_slot", |root| {
        let file_path = root.join("alpha.txt");
        fs::write(&file_path, "alpha\n").expect("failed to write file");

        let session = EditorSession::open_initial_file(&file_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::AssignPinSlot { slot: 3 }, 80, 24);

        let canonical = fs::canonicalize(&file_path).expect("canonical path");
        assert_eq!(
            state.pin_slots_for_test(),
            vec![None, None, None, Some(canonical.clone()), None,]
        );
        assert_eq!(state.pinned_files_for_test(), vec![canonical]);

        state.apply_input(InputAction::OpenFinder, 80, 24);
        let popup = state.finder_popup().expect("finder popup");
        assert_eq!(popup.entries[0].hotkey.as_deref(), Some("Ctrl+4"));
    });
}

#[test]
fn pin_manager_reorders_and_deletes_existing_pins() {
    with_isolated_launch_env("pin_manager_reorders_and_deletes", |root| {
        let files = (0..3)
            .map(|idx| {
                let path = root.join(format!("pin-{idx}.txt"));
                fs::write(&path, format!("pin-{idx}\n")).expect("failed to write pinned file");
                path
            })
            .collect::<Vec<_>>();

        let session = EditorSession::open_initial_file(&files[0]).expect("failed to open session");
        let mut state = EditorState::new(session);

        for path in &files {
            let _ = state
                .session
                .open_file(path)
                .expect("failed to open pinned file");
            state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
            state.apply_input(InputAction::PinSelectorAssign, 80, 24);
        }

        let _ = state
            .session
            .open_file(&files[0])
            .expect("failed to return to first file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);

        assert_eq!(state.mode, EditorMode::PinSelect);
        let popup = state.pin_selector_popup().expect("pin manager popup");
        assert_eq!(popup.selected, 0);

        state.apply_input(InputAction::PinSelectorMoveNext, 80, 24);
        state.apply_input(InputAction::PinSelectorReorderDown, 80, 24);

        let pinned = state.pinned_files_for_test();
        assert_eq!(
            pinned[0],
            fs::canonicalize(&files[0]).expect("pin 0 canonical")
        );
        assert_eq!(
            pinned[1],
            fs::canonicalize(&files[2]).expect("pin 2 canonical")
        );
        assert_eq!(
            pinned[2],
            fs::canonicalize(&files[1]).expect("pin 1 canonical")
        );

        state.apply_input(InputAction::PinSelectorDeleteSelected, 80, 24);

        let pinned = state.pinned_files_for_test();
        assert_eq!(pinned.len(), 2);
        assert_eq!(
            pinned[0],
            fs::canonicalize(&files[0]).expect("pin 0 canonical after delete")
        );
        assert_eq!(
            pinned[1],
            fs::canonicalize(&files[2]).expect("pin 2 canonical after delete")
        );
        assert_eq!(state.mode, EditorMode::PinSelect);
    });
}

#[test]
fn finder_shows_pins_and_filters_files() {
    with_isolated_launch_env("finder_shows_pins_and_filters_files", |root| {
        let main_path = root.join("src").join("main.rs");
        let lib_path = root.join("src").join("lib.rs");
        let notes_path = root.join("notes.md");
        fs::create_dir_all(main_path.parent().expect("src dir")).expect("failed to create src");
        fs::write(&main_path, "fn main() {}\n").expect("failed to write main");
        fs::write(&lib_path, "pub fn lib() {}\n").expect("failed to write lib");
        fs::write(&notes_path, "# Notes\n").expect("failed to write notes");

        let session =
            EditorSession::open_initial_file(&notes_path).expect("failed to open session");
        let mut state = EditorState::new(session);
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);
        let _ = state
            .session
            .open_file(&main_path)
            .expect("failed to switch file");
        state.apply_input(InputAction::QuickPinCurrentFile, 80, 24);
        state.apply_input(InputAction::PinSelectorAssign, 80, 24);

        state.apply_input(InputAction::OpenFinder, 80, 24);
        let popup = state.finder_popup().expect("finder popup");
        assert!(popup.entries[0].is_pinned);
        assert!(popup.entries[1].is_pinned);
        assert_eq!(popup.selected, popup.entries.len().saturating_sub(1));

        for ch in "sma".chars() {
            state.apply_input(InputAction::FinderChar(ch), 80, 24);
        }
        let popup = wait_for_finder_popup(&mut state, |popup| popup.result_count == 1);
        assert_eq!(popup.result_count, 1);
        assert_eq!(popup.selected, popup.entries.len().saturating_sub(1));
        assert!(
            popup
                .entries
                .iter()
                .any(|entry| entry.label.contains("src/main.rs"))
        );
    });
}

#[test]
fn finder_reopens_from_cache_and_refreshes_stale_paths() {
    with_isolated_launch_env("finder_reopens_from_cache", |root| {
        let keep_path = root.join("keep.rs");
        let stale_path = root.join("stale.rs");
        fs::write(&keep_path, "keep\n").expect("failed to write kept file");
        fs::write(&stale_path, "stale\n").expect("failed to write stale file");

        let session = EditorSession::open_initial_file(&keep_path).expect("failed to open session");
        let mut state = EditorState::new(session);

        state.apply_input(InputAction::OpenFinder, 80, 24);
        wait_for_finder_index_idle(&mut state);
        let popup = state.finder_popup().expect("finder popup");
        assert_eq!(popup.result_count, 2);

        state.apply_input(InputAction::FinderCancel, 80, 24);
        fs::remove_file(&stale_path).expect("failed to remove stale file");

        state.apply_input(InputAction::OpenFinder, 80, 24);
        let cached_popup = state.finder_popup().expect("cached finder popup");
        assert_eq!(cached_popup.result_count, 2);
        assert!(
            cached_popup
                .entries
                .iter()
                .any(|entry| entry.label == "stale.rs")
        );

        wait_for_finder_index_idle(&mut state);
        let refreshed_popup = state.finder_popup().expect("refreshed finder popup");
        assert_eq!(refreshed_popup.result_count, 1);
        assert!(
            refreshed_popup
                .entries
                .iter()
                .all(|entry| entry.label != "stale.rs")
        );
    });
}

#[test]
fn command_history_up_and_down_restore_previous_draft() {
    let path = temp_file_path("command_history_draft");
    let mut state = state_with_text(path.clone(), "hello\n");

    run_command(&mut state, "ls");
    run_command(&mut state, "about");

    enter_command_mode(&mut state);
    state.apply_input(InputAction::CommandChar('e'), 80, 24);
    state.apply_input(InputAction::CommandChar(' '), 80, 24);

    state.apply_input(InputAction::CommandHistoryPrev, 80, 24);
    assert_eq!(state.command_line, "about");

    state.apply_input(InputAction::CommandHistoryPrev, 80, 24);
    assert_eq!(state.command_line, "ls");

    state.apply_input(InputAction::CommandHistoryNext, 80, 24);
    assert_eq!(state.command_line, "about");

    state.apply_input(InputAction::CommandHistoryNext, 80, 24);
    assert_eq!(state.command_line, "e ");

    let _ = fs::remove_file(path);
}

#[test]
fn command_history_editing_recalled_entry_detaches_navigation() {
    let path = temp_file_path("command_history_detach");
    let mut state = state_with_text(path.clone(), "hello\n");

    run_command(&mut state, "ls");
    run_command(&mut state, "about");

    enter_command_mode(&mut state);
    state.command_line = "e".to_string();
    state.apply_input(InputAction::CommandHistoryPrev, 80, 24);
    assert_eq!(state.command_line, "about");

    state.apply_input(InputAction::CommandChar('!'), 80, 24);
    assert_eq!(state.command_line, "about!");

    state.apply_input(InputAction::CommandHistoryNext, 80, 24);
    assert_eq!(state.command_line, "about!");

    let _ = fs::remove_file(path);
}

#[test]
fn command_history_skips_consecutive_duplicates() {
    let path = temp_file_path("command_history_dedupe");
    let mut state = state_with_text(path.clone(), "hello\n");

    run_command(&mut state, "ls");
    run_command(&mut state, "ls");

    enter_command_mode(&mut state);
    state.apply_input(InputAction::CommandHistoryPrev, 80, 24);
    assert_eq!(state.command_line, "ls");
    state.apply_input(InputAction::CommandHistoryPrev, 80, 24);
    assert_eq!(state.command_line, "ls");

    let _ = fs::remove_file(path);
}

#[test]
fn invalidate_render_caches_hides_stale_delimiters_until_worker_result() {
    let mut view = BufferViewState::default();
    let before = TextBuffer::from_str("{ alpha }");
    let after = TextBuffer::from_str("plain text");
    let rust_buffer = TextBuffer::from_str("fn main() {\n    answer();\n}\n");

    view.delimiter_pair_cache
        .install(crate::ui::overlays::compute_delimiter_analysis(&before));
    view.syntax_highlighter
        .replace_cache(SyntaxHighlighter::compute_cache(
            &rust_buffer,
            SyntaxLanguage::Rust,
        ));
    assert_eq!(view.delimiter_pair_cache.get().expect("analysis").len(), 1);
    assert!(view.syntax_highlighter.has_cache_for(SyntaxLanguage::Rust));
    let fresh_scope = view
        .syntax_highlighter
        .active_scope_pair_cached(
            &rust_buffer,
            Some(SyntaxLanguage::Rust),
            view.analysis_version,
            Pos::new(1, 4),
        )
        .expect("fresh scope");

    let previous_version = view.analysis_version;
    view.invalidate_render_caches();

    assert_ne!(view.analysis_version, previous_version);
    assert!(view.delimiter_pair_cache.has_stale_analysis());
    assert!(!view.delimiter_pair_cache.has_fresh_analysis());
    assert!(view.delimiter_pair_cache.get().is_none());
    assert!(
        view.syntax_highlighter
            .has_stale_cache_for(SyntaxLanguage::Rust)
    );
    assert!(
        view.syntax_highlighter
            .has_any_cache_for(SyntaxLanguage::Rust)
    );
    assert!(!view.syntax_highlighter.has_cache_for(SyntaxLanguage::Rust));
    assert!(
        view.syntax_highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Rust), 0, 1)
            .is_some()
    );
    assert_eq!(
        view.syntax_highlighter.active_scope_pair_cached(
            &rust_buffer,
            Some(SyntaxLanguage::Rust),
            view.analysis_version,
            Pos::new(1, 4),
        ),
        None
    );
    assert_eq!(
        view.syntax_highlighter
            .active_scope_pair_for_display_cached(
                &rust_buffer,
                Some(SyntaxLanguage::Rust),
                view.analysis_version,
                Pos::new(1, 4),
            ),
        Some(fresh_scope)
    );

    view.syntax_highlighter
        .replace_cache(SyntaxHighlighter::compute_cache(
            &rust_buffer,
            SyntaxLanguage::Rust,
        ));
    assert!(view.syntax_highlighter.has_cache_for(SyntaxLanguage::Rust));

    view.delimiter_pair_cache
        .install(crate::ui::overlays::compute_delimiter_analysis(&after));
    assert!(view.delimiter_pair_cache.has_fresh_analysis());
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

    state.apply_analysis_result(analysis::AnalysisResult::Delimiters {
        buffer_id: active_id,
        version: current_version,
        delimiter_analysis: crate::ui::overlays::compute_delimiter_analysis(
            state.session.active_buffer(),
        ),
    });
    state.apply_analysis_result(analysis::AnalysisResult::Delimiters {
        buffer_id: active_id,
        version: current_version.saturating_sub(1),
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

    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 15);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::FindCharBefore('b'),
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
        .cursor = Pos::new(0, 15);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::TillCharBefore('b'),
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 7));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_percent_jumps_between_matching_delimiters() {
    let path = temp_file_path("match_delimiter_motion");
    let mut state = state_with_text(path.clone(), "fn main() {\n    call([x]);\n}\n");

    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 10);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::MatchDelimiter,
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(2, 0));

    state.apply_input(
        InputAction::Motion {
            motion: Motion::MatchDelimiter,
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 10));

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_operate_percent_deletes_matching_delimiter_range() {
    let path = temp_file_path("operate_match_delimiter");
    let mut state = state_with_text(path.clone(), "a(b)c\n");

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
                motion: Motion::MatchDelimiter,
                count: 1,
            },
        },
        80,
        24,
    );

    assert_eq!(state.session.active_buffer().to_string(), "ac\n");
    assert_eq!(state.private_register, "(b)");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));

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
fn till_char_before_search_repeat_tracks_landing_after_the_match() {
    let path = temp_file_path("till_char_before_search_repeat_target");
    let mut state = state_with_text(path.clone(), "abacadax\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 6);

    state.apply_input(
        InputAction::Motion {
            motion: Motion::TillCharBefore('a'),
            count: 1,
        },
        80,
        24,
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 5));

    state.apply_input(InputAction::RepeatSearch { forward: true }, 80, 24);
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 7));

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
fn search_reports_match_count_in_status_message() {
    let path = temp_file_path("slash_search_match_count");
    let mut state = state_with_text(path.clone(), "test alpha\ntest beta\ngamma test\n");

    state.apply_input(InputAction::EnterSearch, 80, 24);
    for ch in "test".chars() {
        state.apply_input(InputAction::SearchChar(ch), 80, 24);
    }
    state.apply_input(InputAction::SearchEnter, 80, 24);

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.status_msg.as_deref(), Some("3 instances of 'test'"));

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
fn normal_mode_tilde_toggles_case_and_moves_right() {
    let path = temp_file_path("toggle_case_normal");
    let mut state = state_with_text(path.clone(), "aBc\n");

    state.apply_input(InputAction::ToggleCase { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "ABc\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.session.active_meta().dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_tilde_count_toggles_letters_and_steps_over_non_letters() {
    let path = temp_file_path("toggle_case_count");
    let mut state = state_with_text(path.clone(), "a-Bc\n");

    state.apply_input(InputAction::ToggleCase { count: 4 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "A-bC\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 3));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_tilde_on_non_letter_only_moves_cursor() {
    let path = temp_file_path("toggle_case_non_letter");
    let mut state = state_with_text(path.clone(), "-a\n");

    state.apply_input(InputAction::ToggleCase { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "-a\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 1));
    assert!(!state.session.active_meta().dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_tilde_advances_past_multicodepoint_case_expansion() {
    let path = temp_file_path("toggle_case_multicodepoint");
    let mut state = state_with_text(path.clone(), "ßa\n");

    state.apply_input(InputAction::ToggleCase { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "SSa\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 2));
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(state.session.active_meta().dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_char_tilde_toggles_entire_selection_and_normalizes_mode() {
    let path = temp_file_path("toggle_case_visual_char");
    let mut state = state_with_text(path.clone(), "aBcD\n");

    state.apply_input(InputAction::SetMode(InputMode::Visual), 80, 24);
    state.apply_input(
        InputAction::Motion {
            motion: Motion::Right,
            count: 2,
        },
        80,
        24,
    );
    state.apply_input(InputAction::ToggleCase { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "AbCD\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(0, 0));
    assert_eq!(state.mode, EditorMode::Normal);

    let _ = fs::remove_file(path);
}

#[test]
fn visual_block_tilde_toggles_rectangular_selection() {
    let path = temp_file_path("toggle_case_visual_block");
    let mut state = state_with_text(path.clone(), "abC\nDeF\n");

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
    state.apply_input(InputAction::ToggleCase { count: 1 }, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "ABC\ndEF\n");
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
fn command_ls_populates_multiline_status_summary() {
    let path_a = temp_file_path("ls_a");
    let path_b = temp_file_path("ls_b");
    let mut state = state_with_text(path_a.clone(), "alpha");
    fs::write(&path_b, "bravo").expect("failed to write test file");

    state.apply_input(InputAction::Paste("!".to_string()), 80, 24);
    run_command(&mut state, &format!("e {}", path_b.display()));
    run_command(&mut state, "ls");

    let msg = state.status_msg.as_deref().expect("missing ls status");
    assert!(msg.contains("%"));
    assert!(msg.contains("+"));
    assert!(msg.contains('\n'));
    for summary in state.session.summaries() {
        assert!(msg.contains(&summary.display_name));
    }

    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn command_edit_replaces_empty_startup_buffer() {
    with_global_test_state_lock(|| {
        let path = temp_file_path("edit_replaces_empty_startup");
        fs::write(&path, "alpha").expect("failed to write fixture");
        let session =
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
        let placeholder_id = session.active_id();
        let mut state = EditorState::new(session);

        run_command(&mut state, &format!("e {}", path.display()));

        assert!(state.session.buffer(placeholder_id).is_none());
        assert_eq!(state.session.summaries().len(), 1);
        let expected_path = fs::canonicalize(&path).unwrap_or(path.clone());
        assert_eq!(
            state.session.active_meta().path.as_ref(),
            Some(&expected_path)
        );

        run_command(&mut state, "bn");
        assert_eq!(state.status_msg.as_deref(), Some("only one buffer"));

        let _ = fs::remove_file(path);
    });
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
fn command_perf_toggles_performance_popup() {
    let path = temp_file_path("perf_popup");
    let mut state = state_with_text(path.clone(), "let perf = true;\n");

    assert!(state.perf_popup().is_none());
    run_command(&mut state, "perf");

    assert!(state.perf_popup().is_some());
    assert_eq!(state.mode, EditorMode::Normal);

    run_command(&mut state, "perf");

    assert!(state.perf_popup().is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn perf_popup_escape_dismisses_in_normal_mode() {
    let path = temp_file_path("perf_escape");
    let mut state = state_with_text(path.clone(), "let perf = true;\n");

    run_command(&mut state, "perf");

    assert!(state.dismiss_perf_popup());
    assert!(state.perf_popup().is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_ls_status_expires_after_timeout() {
    let path = temp_file_path("ls_ephemeral");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "ls");
    assert!(state.status_msg.is_some());

    expire_status_after_timeout(&mut state);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_ls_status_survives_input_before_timeout() {
    let path = temp_file_path("ls_ephemeral_input");
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

    assert!(state.status_msg.is_some());

    let _ = fs::remove_file(path);
}

#[test]
fn command_write_status_expires_after_timeout() {
    let path = temp_file_path("write_status_clears");
    let mut state = state_with_text(path.clone(), "alpha");

    run_command(&mut state, "w");
    assert_eq!(state.status_msg.as_deref(), Some("written"));

    expire_status_after_timeout(&mut state);
    assert!(state.status_msg.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn command_write_trims_trailing_whitespace_on_save() {
    let path = temp_file_path("write_trims_trailing_whitespace");
    let mut state = state_with_text(path.clone(), "alpha  \nbeta\t \n gamma\t\t\n");

    run_command(&mut state, "w");

    assert_eq!(
        state.session.active_buffer().to_string(),
        "alpha\nbeta\n gamma\n"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("failed to read saved file"),
        "alpha\nbeta\n gamma\n"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn command_write_clamps_cursor_after_trimming_trailing_whitespace() {
    let path = temp_file_path("write_trim_cursor");
    let mut state = state_with_text(path.clone(), "alpha   \n");
    let active_id = state.session.active_id();
    state
        .views
        .get_mut(&active_id)
        .expect("missing active view")
        .cursor
        .cursor = Pos::new(0, 7);

    run_command(&mut state, "w");

    assert_eq!(state.active_cursor_pos(), Pos::new(0, 4));

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
fn command_e_requests_syntax_analysis_for_opened_rust_file() {
    let path_a = temp_file_path("e_analysis_a");
    let path_b = temp_file_path("e_analysis_b").with_extension("rs");
    let mut state = state_with_text(path_a.clone(), "alpha");
    fs::write(&path_b, "fn main() {\n    println!(\"hi\");\n}\n")
        .expect("failed to write test file");

    run_command(&mut state, &format!("e {}", path_b.display()));
    let rust_id = state.session.active_id();
    wait_for_rust_syntax_cache(&mut state, rust_id);

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

    assert_eq!(
        state.status_msg.as_deref(),
        Some("renamed files: a.txt -> renamed.txt, open.txt -> created.txt")
    );
    assert!(dir.join("renamed.txt").exists());
    assert!(dir.join("created.txt").exists());
    assert!(!dir.join("a.txt").exists());

    let _ = fs::remove_file(dir.join("renamed.txt"));
    let _ = fs::remove_file(dir.join("created.txt"));
    let _ = fs::remove_file(file_open);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_creates_nested_paths() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_nested_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);

    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nfolder/nested.txt\ndeep/tree/");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    assert_eq!(
        state.status_msg.as_deref(),
        Some("created directory: deep/tree/; renamed file: open.txt -> folder/nested.txt")
    );
    assert!(dir.join("folder").is_dir());
    assert!(dir.join("folder/nested.txt").exists());
    assert!(dir.join("deep/tree").is_dir());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn explorer_preserves_unsaved_directory_draft_across_navigation() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_preserve_draft_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let child_dir = dir.join("child");
    fs::create_dir(&child_dir).expect("failed to create child dir");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");
    fs::write(child_dir.join("keep.txt"), "keep").expect("failed to write child fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nchild/\nopen.txt\nroot_new.txt");
    }
    let _ = state.session.recompute_active_dirty();

    let explorer_id = state.session.active_id();
    let child_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "child/")
        .expect("child directory missing from explorer");
    state
        .views
        .get_mut(&explorer_id)
        .expect("missing explorer view")
        .cursor
        .cursor = Pos::new(child_line, 0);
    state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);
    state.surface_go_parent();

    assert!(
        state
            .session
            .active_buffer()
            .to_string()
            .contains("root_new.txt")
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn explorer_write_applies_cached_drafts_from_multiple_directories() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_multi_dir_write_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let child_dir = dir.join("child");
    fs::create_dir(&child_dir).expect("failed to create child dir");
    let file_open = dir.join("open.txt");
    fs::write(&file_open, "open").expect("failed to write fixture");
    fs::write(child_dir.join("keep.txt"), "keep").expect("failed to write child fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nchild/\nopen.txt\nroot_new.txt");
    }
    let _ = state.session.recompute_active_dirty();

    let explorer_id = state.session.active_id();
    let child_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "child/")
        .expect("child directory missing from explorer");
    state
        .views
        .get_mut(&explorer_id)
        .expect("missing explorer view")
        .cursor
        .cursor = Pos::new(child_line, 0);
    state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nkeep.txt\nchild_new.txt");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    assert!(dir.join("root_new.txt").exists());
    assert!(child_dir.join("child_new.txt").exists());
    assert_eq!(
        state.status_msg.as_deref(),
        Some("created files: root_new.txt, child_new.txt")
    );

    let _ = fs::remove_dir_all(dir);
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
    assert!(msg.contains("z_delete.txt"));
    assert_eq!(
        state.status_msg_line_styles,
        vec![
            StatusMessageStyle::Normal,
            StatusMessageStyle::Normal,
            StatusMessageStyle::Dim
        ]
    );
    assert!(state.status_message_is_sticky());

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
    assert_eq!(
        state.status_msg.as_deref(),
        Some("deleted file: z_delete.txt")
    );

    let _ = fs::remove_file(file_keep);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_delete_file_and_create_directory_still_requires_delete_confirmation() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_delete_create_kind_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let file_keep = dir.join("keep.txt");
    let file_delete = dir.join("delete.txt");
    fs::write(&file_keep, "keep").expect("failed to write fixture");
    fs::write(&file_delete, "delete").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_keep).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");
    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nkeep.txt\nfresh/");
    }
    let _ = state.session.recompute_active_dirty();

    run_command(&mut state, "w");

    assert!(file_delete.exists());
    assert!(!dir.join("fresh").exists());
    let msg = state
        .status_msg
        .as_deref()
        .expect("missing confirmation prompt");
    assert!(msg.contains("confirm deletion of 1 entry"));
    assert!(msg.contains("delete.txt"));

    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);

    assert_eq!(
        state.status_msg.as_deref(),
        Some("created directory: fresh/; deleted file: delete.txt")
    );
    assert!(!file_delete.exists());
    assert!(dir.join("fresh").is_dir());

    let _ = fs::remove_file(file_keep);
    let _ = fs::remove_dir_all(dir);
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
    assert!(msg.contains("confirm deletion of 2 entries"));
    assert!(msg.contains("nested/"));
    assert!(msg.contains("nested/child.txt"));
    assert!(msg.contains("\n nested/"));
    assert!(msg.contains("\npress y"));

    state.apply_input(InputAction::ConfirmExplorerDelete, 80, 24);
    assert!(!doomed_dir.exists());
    assert_eq!(
        state.status_msg.as_deref(),
        Some("deleted directory: nested/")
    );

    let _ = fs::remove_file(file_keep);
    let _ = fs::remove_dir(dir);
}

#[test]
fn explorer_write_multi_directory_delete_confirmation_includes_directory_context() {
    let dir = std::env::temp_dir().join(format!(
        "redox_explorer_multi_dir_confirm_delete_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    fs::create_dir(&dir).expect("failed to create temp dir");
    let child_dir = dir.join("child");
    fs::create_dir(&child_dir).expect("failed to create child dir");
    let file_open = dir.join("open.txt");
    let root_delete = dir.join("duplicate.txt");
    let child_delete = child_dir.join("duplicate.txt");
    fs::write(&file_open, "open").expect("failed to write open fixture");
    fs::write(&root_delete, "root").expect("failed to write root fixture");
    fs::write(&child_delete, "child").expect("failed to write child fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../\nchild/\nopen.txt");
    }
    let _ = state.session.recompute_active_dirty();

    let explorer_id = state.session.active_id();
    let child_line = state
        .session
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line == "child/")
        .expect("child directory missing from explorer");
    state
        .views
        .get_mut(&explorer_id)
        .expect("missing explorer view")
        .cursor
        .cursor = Pos::new(child_line, 0);
    state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

    {
        let buffer = state.session.active_buffer_mut();
        *buffer = TextBuffer::from_str("../");
    }
    let _ = state.session.recompute_active_dirty();

    state.surface_go_parent();
    run_command(&mut state, "w");

    let msg = state
        .status_msg
        .as_deref()
        .expect("missing confirmation prompt");
    assert!(msg.contains("confirm deletion of 2 entries"));
    assert!(msg.contains("./duplicate.txt"));
    assert!(msg.contains("child/duplicate.txt"));

    let _ = fs::remove_file(file_open);
    let _ = fs::remove_file(root_delete);
    let _ = fs::remove_file(child_delete);
    let _ = fs::remove_dir(child_dir);
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
    with_global_test_state_lock(|| {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_single_q_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        fs::write(dir.join("a.txt"), "alpha").expect("failed to write fixture");

        let session =
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        state
            .open_explorer_at_path(dir.clone())
            .expect("failed to open explorer");
        assert!(state.explorer_popup().is_some());

        run_command(&mut state, "q");

        assert!(state.should_quit);

        let _ = fs::remove_dir_all(dir);
    });
}

#[test]
fn explorer_open_at_dot_resolves_title_to_real_directory_path() {
    with_global_test_state_lock(|| {
        let session =
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
        let mut state = EditorState::new(session);

        state
            .open_explorer_at_path(PathBuf::from("."))
            .expect("failed to open explorer at dot");
        let popup = state
            .explorer_popup()
            .expect("explorer popup should be active");

        assert!(!popup.title.starts_with("~./"));
        assert!(popup.title.ends_with('/'));
    });
}

#[test]
fn explorer_directory_launch_marks_background_as_placeholder_blank() {
    with_global_test_state_lock(|| {
        let session =
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
        let mut state = EditorState::new(session);

        state
            .open_explorer_at_path(PathBuf::from("."))
            .expect("failed to open explorer at dot");

        assert!(state.explorer_background_is_placeholder_blank());
    });
}

#[test]
fn explorer_file_open_replaces_empty_startup_background_buffer() {
    with_global_test_state_lock(|| {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_file_replaces_startup_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        let file_path = dir.join("a.txt");
        fs::write(&file_path, "alpha").expect("failed to write fixture");

        let session =
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session");
        let placeholder_id = session.active_id();
        let mut state = EditorState::new(session);
        state
            .open_explorer_at_path(dir.clone())
            .expect("failed to open explorer");
        assert!(state.explorer_background_is_placeholder_blank());

        let explorer_id = state.session.active_id();
        let file_line = state
            .session
            .active_buffer()
            .to_string()
            .lines()
            .position(|line| line == "a.txt")
            .expect("file missing from explorer");
        state
            .views
            .get_mut(&explorer_id)
            .expect("missing explorer view")
            .cursor
            .cursor = Pos::new(file_line, 0);

        state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

        assert!(state.session.buffer(placeholder_id).is_none());
        assert!(state.session.buffer(explorer_id).is_none());
        assert_eq!(state.session.summaries().len(), 1);
        let expected_path = fs::canonicalize(&file_path).unwrap_or(file_path.clone());
        assert_eq!(
            state.session.active_meta().path.as_ref(),
            Some(&expected_path)
        );

        run_command(&mut state, "bp");
        assert_eq!(state.status_msg.as_deref(), Some("only one buffer"));

        let _ = fs::remove_dir_all(dir);
    });
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
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);

        run_command(&mut state, "about");
        assert!(state.about_popup().is_some());

        run_command(&mut state, "q");

        assert!(state.should_quit);
        assert!(state.about_popup().is_none());
    });
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
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);

        run_command(&mut state, "about");
        assert!(state.about_popup().is_some());

        assert!(state.handle_normal_mode_escape_on_surface());

        assert!(state.should_quit);
        assert!(state.about_popup().is_none());
    });
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
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state.command_open_about();
        let about_id = state.session.active_id();
        assert!(state.about_popup().is_some());

        run_command(&mut state, "explorer");

        assert!(!state.should_quit);
        assert!(state.about_popup().is_none());
        assert!(state.explorer_popup().is_some());
        assert_ne!(state.explorer_background_buffer_id(), Some(about_id));
        assert!(state.explorer_background_is_placeholder_blank());
    });
}

#[test]
fn explorer_escape_from_startup_about_returns_to_about() {
    with_global_test_state_lock(|| {
        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let mut state = EditorState::new(session);
        state.command_open_about();
        assert!(state.about_popup().is_some());

        run_command(&mut state, "explorer");
        assert!(state.explorer_popup().is_some());

        assert!(state.handle_normal_mode_escape_on_surface());

        assert!(!state.should_quit);
        assert!(state.explorer_popup().is_none());
        assert!(state.about_popup().is_some());
    });
}

#[test]
fn explorer_file_open_from_startup_about_replaces_empty_startup_buffer() {
    with_global_test_state_lock(|| {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_about_file_replaces_startup_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        let file_path = dir.join("a.txt");
        fs::write(&file_path, "alpha").expect("failed to write fixture");

        let session = EditorSession::open_initial_unnamed().expect("failed to open session");
        let placeholder_id = session.active_id();
        let mut state = EditorState::new(session);
        state.command_open_about();
        let about_id = state.session.active_id();
        assert!(state.about_popup().is_some());

        state
            .open_explorer_at_path(dir.clone())
            .expect("failed to open explorer");
        assert!(state.about_popup().is_none());
        assert!(state.explorer_popup().is_some());
        assert_ne!(state.explorer_background_buffer_id(), Some(about_id));
        assert!(state.explorer_background_is_placeholder_blank());

        let explorer_id = state.session.active_id();
        let file_line = state
            .session
            .active_buffer()
            .to_string()
            .lines()
            .position(|line| line == "a.txt")
            .expect("file missing from explorer");
        state
            .views
            .get_mut(&explorer_id)
            .expect("missing explorer view")
            .cursor
            .cursor = Pos::new(file_line, 0);

        state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

        assert!(state.session.buffer(placeholder_id).is_none());
        assert!(state.session.buffer(explorer_id).is_none());
        assert_eq!(state.session.summaries().len(), 2);
        let expected_path = fs::canonicalize(&file_path).unwrap_or(file_path.clone());
        assert_eq!(
            state.session.active_meta().path.as_ref(),
            Some(&expected_path)
        );

        run_command(&mut state, "ls");
        let msg = state.status_msg.as_deref().expect("missing ls status");
        assert!(!msg.contains("[No Name]"));
        assert!(!msg.contains(" | "));

        let _ = fs::remove_dir_all(dir);
    });
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
    let file_a = dir.join("a.rs");
    let file_open = dir.join("open.txt");
    fs::write(&file_a, "fn main() {}\n").expect("failed to write fixture");
    fs::write(&file_open, "open").expect("failed to write fixture");

    let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
    let mut state = EditorState::new(session);
    run_command(&mut state, "explorer");

    {
        let text = state.session.active_buffer().to_string();
        let target_line = text
            .lines()
            .position(|line| line == "a.rs")
            .expect("a.rs missing from explorer listing");
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
    assert_eq!(state.session.active_buffer().to_string(), "fn main() {}\n");
    let rust_id = state.session.active_id();
    wait_for_rust_syntax_cache(&mut state, rust_id);

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
fn normal_mode_cc_preserves_current_line_indent() {
    let path = temp_file_path("cc_preserve_indent");
    let mut state = state_with_text(path.clone(), "one\n\ttwo\nthree\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(1, 2);

    state.apply_input(InputAction::ChangeCurrentLinePrivate { count: 1 }, 80, 24);

    assert_eq!(state.private_register, "\ttwo\n");
    assert_eq!(
        state.session.active_buffer().to_string(),
        "one\n\t\nthree\n"
    );
    assert_eq!(state.mode, EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn insert_enter_uses_tree_sitter_smart_indent() {
    let path = temp_file_path("smart_enter").with_extension("rs");
    let mut state = state_with_text(path.clone(), "fn main() {");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 11);
    state.apply_input(InputAction::EnterInsert(InsertKind::AppendLineEnd), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "fn main() {\n\t");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn insert_enter_splits_closing_delimiter_with_smart_indent() {
    let path = temp_file_path("smart_enter_closing").with_extension("rs");
    let mut state = state_with_text(path.clone(), "fn main() {}");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 11);
    state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\t\n}"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn insert_enter_splits_angle_delimiters_with_smart_indent() {
    let path = temp_file_path("smart_enter_angle").with_extension("html");
    let mut state = state_with_text(path.clone(), "<>");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::EnterInsert(InsertKind::Append), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "<\n\t\n>");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn insert_enter_splits_quote_delimiters_with_smart_indent() {
    let path = temp_file_path("smart_enter_quotes").with_extension("rs");
    let mut state = state_with_text(path.clone(), "let text = \"\";");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 11);
    state.apply_input(InputAction::EnterInsert(InsertKind::Append), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(
        state.session.active_buffer().to_string(),
        "let text = \"\n\t\n\";"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn insert_enter_splits_backtick_delimiters_with_smart_indent() {
    // Backtick smart-enter behaviour relies on backticks staying paired delimiters.
    let path = temp_file_path("smart_enter_backticks").with_extension("md");
    let mut state = state_with_text(path.clone(), "``");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::EnterInsert(InsertKind::Append), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "`\n\t\n`");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));
    let _ = fs::remove_file(path);
}

#[test]
fn normal_mode_o_and_shift_o_indent_between_delimiters() {
    let path = temp_file_path("smart_open_line").with_extension("rs");
    let mut state = state_with_text(path.clone(), "fn main() {\n}\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);

    state.apply_input(InputAction::OpenLineBelow, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\t\n}\n"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));

    state.apply_input(InputAction::SetMode(InputMode::Normal), 80, 24);
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(2, 0);
    state.apply_input(InputAction::OpenLineAbove, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\t\n\t\n}\n"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(2, 1));

    let _ = fs::remove_file(path);
}

#[test]
fn markdown_enter_and_open_line_continue_list_indentation() {
    let path = temp_file_path("smart_markdown").with_extension("md");
    let mut state = state_with_text(path.clone(), "- item");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 6);
    state.apply_input(InputAction::EnterInsert(InsertKind::AppendLineEnd), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "- item\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 0));

    state.apply_input(InputAction::SetMode(InputMode::Normal), 80, 24);
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 0);
    state.apply_input(InputAction::OpenLineBelow, 80, 24);
    assert_eq!(state.session.active_buffer().to_string(), "- item\n\n");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn smart_indent_does_not_skip_over_blank_lines() {
    let path = temp_file_path("smart_indent_blank").with_extension("rs");
    let mut state = state_with_text(path.clone(), "fn main() {\n\tcall();\n\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(2, 0);
    state.apply_input(InputAction::EnterInsert(InsertKind::AppendLineEnd), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\tcall();\n\n\n"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(3, 0));

    let _ = fs::remove_file(path);
}

#[test]
fn smart_indent_floors_partial_tab_widths() {
    let path = temp_file_path("smart_indent_floor").with_extension("rs");
    let mut state = state_with_text(path.clone(), "\t  if ready {");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 13);
    state.apply_input(InputAction::EnterInsert(InsertKind::AppendLineEnd), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(
        state.session.active_buffer().to_string(),
        "\t  if ready {\n\t\t"
    );
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 2));

    let _ = fs::remove_file(path);
}

#[test]
fn markdown_list_indent_floors_to_full_tab_width() {
    let path = temp_file_path("smart_markdown_floor").with_extension("md");
    let mut state = state_with_text(path.clone(), "    - item");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(0, 10);
    state.apply_input(InputAction::EnterInsert(InsertKind::AppendLineEnd), 80, 24);

    state.apply_input(InputAction::Enter, 80, 24);

    assert_eq!(state.session.active_buffer().to_string(), "    - item\n\t");
    assert_eq!(state.active_cursor_pos(), Pos::new(1, 1));

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
fn visual_move_reindents_line_for_new_tree_sitter_scope() {
    let path = temp_file_path("visual_move_smart_indent").with_extension("rs");
    let mut state = state_with_text(path.clone(), "fn main() {\n\tcall();\n}\nlet x = 1;\n");
    let id = state.session.active_id();
    state
        .views
        .get_mut(&id)
        .expect("missing view")
        .cursor
        .cursor = Pos::new(3, 0);
    state.apply_input(InputAction::SetMode(InputMode::VisualLine), 80, 24);

    state.apply_input(InputAction::MoveVisualSelectionUp { count: 1 }, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\tcall();\n\tlet x = 1;\n}\n"
    );

    state.apply_input(InputAction::MoveVisualSelectionDown { count: 1 }, 80, 24);
    assert_eq!(
        state.session.active_buffer().to_string(),
        "fn main() {\n\tcall();\n}\nlet x = 1;\n"
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
