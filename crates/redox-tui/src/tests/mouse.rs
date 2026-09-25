use super::*;
use crate::app::EditorMode;
use crate::ui::widgets::popup::{MousePopup, MouseRect, MouseScroll, MouseTarget};
use minui::MouseButton;
use redox_core::Pos;
use std::ffi::OsString;
use std::path::PathBuf;

struct MouseFiles {
    root: PathBuf,
    previous_directory: PathBuf,
    previous_config: Option<OsString>,
    previous_state: Option<OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl MouseFiles {
    fn new() -> Self {
        let lock = app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temp_dir_path("popup_mouse");
        fs::create_dir_all(root.join("project")).unwrap();
        let fixture = Self {
            root,
            previous_directory: std::env::current_dir().unwrap(),
            previous_config: std::env::var_os("XDG_CONFIG_HOME"),
            previous_state: std::env::var_os("XDG_STATE_HOME"),
            _lock: lock,
        };
        std::env::set_current_dir(fixture.root.join("project")).unwrap();
        // Match the existing isolated launch tests; all environment changes hold
        // the shared lock and are restored even when an assertion fails.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", fixture.root.join("config"));
            std::env::set_var("XDG_STATE_HOME", fixture.root.join("state"));
        }
        fixture
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.root.join("project").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        fs::canonicalize(path).unwrap()
    }
}

impl Drop for MouseFiles {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous_directory).unwrap();
        for (name, previous) in [
            ("XDG_CONFIG_HOME", &self.previous_config),
            ("XDG_STATE_HOME", &self.previous_state),
        ] {
            unsafe {
                match previous {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn render(state: &mut EditorState, window: &mut TestWindow) {
    window.clear_screen().unwrap();
    draw_buffer_view(
        state,
        UiStyle::default(),
        window,
        &mut FramePerfSample::default(),
    )
    .unwrap();
}

fn click(state: &mut EditorState, window: &mut TestWindow, x: u16, y: u16) {
    handle_editor_event(
        state,
        &mut None,
        Event::MouseClick {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    handle_editor_event(
        state,
        &mut None,
        Event::MouseRelease {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    render(state, window);
}

fn target_rect(state: &EditorState, popup: MousePopup, target: &MouseTarget) -> MouseRect {
    state
        .mouse
        .popups
        .iter()
        .find(|layout| layout.kind == popup)
        .unwrap()
        .clicks
        .iter()
        .find(|(_, candidate)| candidate == target)
        .unwrap_or_else(|| panic!("missing target {target:?}"))
        .0
}

fn scroll_at(state: &mut EditorState, window: &mut TestWindow, region: MouseScroll, delta: i8) {
    let rect = state
        .mouse
        .popups
        .last()
        .unwrap()
        .scrolls
        .iter()
        .find(|(_, candidate)| *candidate == region)
        .unwrap()
        .0;
    handle_editor_event(
        state,
        &mut None,
        Event::MouseScroll {
            x: rect.x + 1,
            y: rect.y + 1,
            delta,
        },
    );
    render(state, window);
}

fn wait_for_finder(state: &mut EditorState, file_count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.finder_popup().unwrap().total_count < file_count {
        assert!(
            Instant::now() < deadline,
            "finder indexing did not complete"
        );
        state.poll_finder_results();
        std::thread::yield_now();
    }
}

#[test]
fn finder_mouse_uses_drawn_rows_and_pointer_region_then_opens_on_double_click() {
    let files = MouseFiles::new();
    let alpha = files.file(
        "alpha.txt",
        &format!("{}\n", "preview ".repeat(30)).repeat(80),
    );
    let beta = files.file("beta.txt", "beta\n");
    for name in ["charlie", "delta", "echo", "foxtrot"] {
        files.file(&format!("{name}.txt"), "text\n");
    }
    let mut state = EditorState::new(EditorSession::open_initial_file(&alpha).unwrap());
    state.configure_mouse(true, false, false);
    state.apply_input(InputAction::QuickPinCurrentFile, 160, 40);
    state.apply_input(InputAction::AssignPinSlot { slot: 0 }, 160, 40);
    state.apply_input(InputAction::OpenFinder, 160, 40);
    wait_for_finder(&mut state, 6);
    let mut window = TestWindow::new(160, 40);
    render(&mut state, &mut window);

    let layout = state.mouse.popups.last().unwrap();
    let list = layout
        .scrolls
        .iter()
        .find(|(_, kind)| *kind == MouseScroll::List)
        .unwrap()
        .0;
    let blank_row = (list.y + 1..list.y + list.height - 1)
        .find(|row| {
            !layout
                .clicks
                .iter()
                .any(|(rect, _)| rect.contains_point(list.x + 1, *row))
        })
        .expect("finder leaves blank rows between pins and bottom-aligned files");
    let selected = state.finder_popup().unwrap().selected;
    click(&mut state, &mut window, list.x + 1, blank_row);
    assert_eq!(state.finder_popup().unwrap().selected, selected);

    let pinned = target_rect(
        &state,
        MousePopup::Finder,
        &MouseTarget::FinderEntry(alpha.clone()),
    );
    assert!(window.row_text(pinned.y).contains("alpha.txt"));
    click(&mut state, &mut window, pinned.x, pinned.y);
    assert_eq!(state.finder_popup().unwrap().selected, 0);
    scroll_at(&mut state, &mut window, MouseScroll::List, 1);
    assert_eq!(
        state.finder_popup().unwrap().selected,
        0,
        "a list that already fits must not move its selection when scrolled"
    );
    state.configure_mouse(true, true, false);
    scroll_at(&mut state, &mut window, MouseScroll::List, 1);
    assert_eq!(state.finder_popup().unwrap().selected, 0);
    state.configure_mouse(true, false, false);
    scroll_at(&mut state, &mut window, MouseScroll::Preview, 1);
    let popup = state.finder_popup().unwrap();
    assert_eq!(
        popup.selected, 0,
        "preview scrolling must not change the list selection"
    );
    assert_eq!(popup.preview.unwrap().scroll_y, 3);

    let entry = target_rect(
        &state,
        MousePopup::Finder,
        &MouseTarget::FinderEntry(beta.clone()),
    );
    assert!(window.row_text(entry.y).contains("beta.txt"));
    click(&mut state, &mut window, entry.x, entry.y);
    let popup = state.finder_popup().unwrap();
    assert_eq!(popup.entries[popup.selected].path, beta);
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(alpha.as_path())
    );
    click(&mut state, &mut window, entry.x, entry.y);
    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(beta.as_path())
    );

    state.apply_input(InputAction::OpenFinder, 160, 40);
    render(&mut state, &mut window);
    let cursor = state.active_cursor_pos();
    click(&mut state, &mut window, 0, 0);
    assert!(state.finder_popup().is_none());
    assert_eq!(
        state.active_cursor_pos(),
        cursor,
        "closing must not click through to the editor"
    );
}

#[test]
fn finder_scrolls_the_page_and_keeps_physical_double_click_targets_still() {
    let files = MouseFiles::new();
    let paths: Vec<_> = (0..50)
        .map(|index| files.file(&format!("entry-{index:02}.txt"), "text\n"))
        .collect();
    let mut state = EditorState::new(EditorSession::open_initial_file(&paths[0]).unwrap());
    state.configure_mouse(true, false, false);
    state.apply_input(InputAction::OpenFinder, 100, 24);
    wait_for_finder(&mut state, paths.len());
    let mut window = TestWindow::new(100, 24);
    render(&mut state, &mut window);
    let visible = |state: &EditorState| {
        state
            .mouse
            .popups
            .iter()
            .find(|popup| popup.kind == MousePopup::Finder)
            .unwrap()
            .clicks
            .iter()
            .filter_map(|(rect, target)| match target {
                MouseTarget::FinderEntry(path) => Some((*rect, path.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let ordered_paths: Vec<_> = state
        .finder_popup()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    let entry_index = |path: &PathBuf| {
        ordered_paths
            .iter()
            .position(|candidate| candidate == path)
            .unwrap()
    };
    let before = visible(&state);
    assert!(before.len() >= 7 && before.len() < paths.len());
    let first_before = entry_index(&before[0].1);
    let (middle_rect, middle_path) = before[before.len() / 2].clone();
    click(&mut state, &mut window, middle_rect.x, middle_rect.y);
    assert_eq!(
        target_rect(
            &state,
            MousePopup::Finder,
            &MouseTarget::FinderEntry(middle_path.clone())
        )
        .y,
        middle_rect.y,
        "selecting a row must not recenter the list under the pointer"
    );
    let selected = state.finder_popup().unwrap().selected;
    scroll_at(&mut state, &mut window, MouseScroll::List, -1);
    assert_eq!(entry_index(&visible(&state)[0].1), first_before - 3);
    assert_eq!(state.finder_popup().unwrap().selected, selected);

    state.configure_mouse(true, true, false);
    scroll_at(&mut state, &mut window, MouseScroll::List, -1);
    assert_eq!(entry_index(&visible(&state)[0].1), first_before);
    assert_eq!(state.finder_popup().unwrap().selected, selected);
    state.configure_mouse(true, false, false);
    for _ in 0..2 {
        scroll_at(&mut state, &mut window, MouseScroll::List, -1);
    }
    let after = visible(&state);
    assert_eq!(entry_index(&after[0].1), first_before - 6);
    let popup = state.finder_popup().unwrap();
    assert_eq!(
        popup.entries[popup.selected].path,
        after.last().unwrap().1,
        "the selected file moves only once it reaches the viewport edge"
    );
    click(&mut state, &mut window, 0, 0);

    for click_top in [true, false] {
        state.apply_input(InputAction::OpenFinder, 100, 24);
        wait_for_finder(&mut state, paths.len());
        render(&mut state, &mut window);
        scroll_at(&mut state, &mut window, MouseScroll::List, -1);
        let rows = visible(&state);
        let (rect, path) = if click_top { rows.first() } else { rows.last() }
            .unwrap()
            .clone();
        click(&mut state, &mut window, rect.x, rect.y);
        // MinUI discards unsupported input and wheel noise as Unknown. Neither
        // may reset the list window or interrupt the second click.
        handle_editor_event(&mut state, &mut None, Event::Unknown);
        render(&mut state, &mut window);
        let still = target_rect(
            &state,
            MousePopup::Finder,
            &MouseTarget::FinderEntry(path.clone()),
        );
        assert_eq!(
            (still.x, still.y),
            (rect.x, rect.y),
            "click_top={click_top}"
        );
        click(&mut state, &mut window, rect.x, rect.y);
        assert!(state.finder_popup().is_none(), "click_top={click_top}");
        assert_eq!(state.session.active_meta().path.as_ref(), Some(&path));
    }
}

fn explorer_entry(state: &EditorState, name: &str) -> MouseRect {
    let buffer = state.session.active_buffer();
    let line = (0..buffer.len_lines())
        .find(|line| buffer.line_string(*line).trim() == name)
        .unwrap();
    target_rect(
        state,
        MousePopup::Explorer,
        &MouseTarget::BufferPosition(Pos::new(line, 0)),
    )
}

#[test]
fn input_popups_route_cursor_clicks_and_outside_dismissal() {
    let files = MouseFiles::new();
    let document = files.file("search.txt", &"hit\n".repeat(6));
    let mut state = EditorState::new(EditorSession::open_initial_file(&document).unwrap());
    state.configure_mouse(true, false, false);
    let mut window = TestWindow::new(100, 30);
    state.apply_input(InputAction::RunCommand("ls".to_string()), 100, 30);
    state.apply_input(InputAction::EnterCommand, 100, 30);
    for character in "abcde".chars() {
        state.apply_input(InputAction::CommandChar(character), 100, 30);
    }
    render(&mut state, &mut window);
    let position = target_rect(&state, MousePopup::Command, &MouseTarget::InputCursor(2));
    click(&mut state, &mut window, position.x, position.y);
    handle_editor_event(&mut state, &mut None, Event::Character('X'));
    assert_eq!(state.command_line, "abXcde");
    render(&mut state, &mut window);
    click(&mut state, &mut window, 0, 0);
    assert_eq!(state.mode, EditorMode::Normal);

    let origin = state.active_cursor_pos();
    state.apply_input(InputAction::EnterSearch, 100, 30);
    for character in "hit".chars() {
        state.apply_input(InputAction::SearchChar(character), 100, 30);
    }
    state.poll_search_preview(Instant::now() + Duration::from_secs(1));
    render(&mut state, &mut window);
    let (_, count) = state.search_match_position();
    assert_eq!(count, 6);
    click(&mut state, &mut window, 0, 0);
    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(state.active_cursor_pos(), origin);

    state.apply_input(InputAction::RunCommand("about".to_string()), 100, 30);
    render(&mut state, &mut window);
    click(&mut state, &mut window, 0, 0);
    assert_eq!(state.session.active_meta().path.as_ref(), Some(&document));
    state.apply_input(InputAction::RunCommand("perf popup".to_string()), 100, 30);
    state.apply_input(
        InputAction::SetMode(crate::input::InputMode::Insert),
        100,
        30,
    );
    render(&mut state, &mut window);
    assert!(state.perf_popup().is_some());
    click(&mut state, &mut window, 0, 0);
    assert!(state.perf_popup().is_none());
    assert_eq!(state.mode, EditorMode::Insert);
}

#[test]
fn perf_overlay_allows_buffer_scrolling_inside_and_outside_its_bounds() {
    let files = MouseFiles::new();
    let document = files.file(
        "performance.txt",
        &format!("{}\n", "abcdef".repeat(30)).repeat(100),
    );
    for split in [false, true] {
        let mut state = EditorState::new(EditorSession::open_initial_file(&document).unwrap());
        state.configure_mouse(true, false, false);
        if split {
            state.split_active_pane(app::state::SplitAxis::Vertical);
        }
        state.apply_input(InputAction::RunCommand("perf popup".to_string()), 100, 30);
        if split {
            state.apply_input(
                InputAction::SetMode(crate::input::InputMode::Insert),
                100,
                30,
            );
        }
        let mode = state.mode;
        let mut window = TestWindow::new(100, 30);
        render(&mut state, &mut window);
        let frame = state
            .mouse
            .popups
            .iter()
            .find(|popup| popup.kind == MousePopup::Perf)
            .unwrap()
            .frames[0];
        let active = state
            .pane_rects(100, 29)
            .into_iter()
            .find(|pane| pane.pane_id == state.active_pane_id())
            .unwrap();
        let outside = (active.x + 4, 20);
        let inside = (frame.x + 1, frame.y + 1);
        assert!(!frame.contains_point(outside.0, outside.1));
        for (index, (x, y)) in [outside, inside].into_iter().enumerate() {
            handle_editor_event(&mut state, &mut None, Event::MouseScroll { x, y, delta: 1 });
            render(&mut state, &mut window);
            state.with_active_buffer_view_mut(|_, view| {
                assert_eq!(view.cursor.viewport_scroll(), (0, (index + 1) * 3));
            });
            assert!(state.perf_popup().is_some());
        }
        for _ in 0..3 {
            handle_editor_event(
                &mut state,
                &mut None,
                Event::MouseScrollHorizontal {
                    x: inside.0,
                    y: inside.1,
                    delta: 1,
                },
            );
        }
        render(&mut state, &mut window);
        state.with_active_buffer_view_mut(|_, view| {
            assert_eq!(view.cursor.viewport_scroll(), (3, 6))
        });
        assert!(state.perf_popup().is_some());
        assert_eq!(state.mode, mode);

        if state.mode == EditorMode::Insert {
            handle_editor_event(&mut state, &mut None, Event::Escape);
        }
        handle_editor_event(&mut state, &mut None, Event::Character(':'));
        render(&mut state, &mut window);
        let scroll = state.with_active_buffer_view_mut(|_, view| view.cursor.viewport_scroll());
        handle_editor_event(
            &mut state,
            &mut None,
            Event::MouseScroll {
                x: inside.0,
                y: inside.1,
                delta: -1,
            },
        );
        assert!(state.command_line.is_empty());
        state.with_active_buffer_view_mut(|_, view| {
            assert_eq!(view.cursor.viewport_scroll(), scroll)
        });
    }
}

#[test]
fn popup_wheels_and_dismissal_momentum_leave_the_buffer_still() {
    let files = MouseFiles::new();
    let document = files.file(
        "background.txt",
        &format!("{}\n", "abcdef".repeat(30)).repeat(100),
    );
    for action in [
        InputAction::EnterCommand,
        InputAction::EnterSearch,
        InputAction::RunCommand("about".to_string()),
        InputAction::OpenFinder,
    ] {
        for dismiss_with_click in [false, true] {
            let mut state = EditorState::new(EditorSession::open_initial_file(&document).unwrap());
            state.configure_mouse(true, false, false);
            let mut window = TestWindow::new(100, 30);
            render(&mut state, &mut window);
            let source = state.session.active_id();
            state.with_active_buffer_view_mut(|_, view| {
                view.cursor.place_cursor(Pos::new(15, 20));
                view.cursor.scroll_x_cells = 6;
                view.cursor.scroll_y_lines = 10;
            });
            state.sync_active_pane_view();
            state.apply_input(InputAction::RunCommand("ls".to_string()), 100, 30);
            state.apply_input(action.clone(), 100, 30);
            render(&mut state, &mut window);
            let frame = state.mouse.popups.last().unwrap().frames[0];
            let before = state
                .with_buffer_view_mut(source, |_, view| view.cursor.viewport_scroll())
                .unwrap();
            let mut wheels = Vec::new();
            for (x, y) in [(frame.x + 1, frame.y + 1), (0, 0)] {
                for delta in [-1, 1, 1, 1] {
                    wheels.push(Event::MouseScroll { x, y, delta });
                    wheels.push(Event::MouseScrollHorizontal { x, y, delta });
                }
            }
            for event in &wheels {
                handle_editor_event(&mut state, &mut None, event.clone());
                render(&mut state, &mut window);
            }
            if state.mode == EditorMode::Command {
                assert!(state.command_line.is_empty());
            }
            assert_eq!(
                state
                    .with_buffer_view_mut(source, |_, view| view.cursor.viewport_scroll())
                    .unwrap(),
                before
            );
            if dismiss_with_click {
                click(&mut state, &mut window, 0, 0);
            } else {
                handle_editor_event(&mut state, &mut None, Event::Escape);
                render(&mut state, &mut window);
            }
            assert_eq!(state.mode, EditorMode::Normal);
            assert_eq!(state.session.active_id(), source);
            for event in wheels {
                handle_editor_event(&mut state, &mut None, event);
                render(&mut state, &mut window);
            }
            assert_eq!(
                state.with_active_buffer_view_mut(|_, view| view.cursor.viewport_scroll()),
                before
            );
            assert_eq!(state.active_cursor_pos(), Pos::new(15, 20));
        }
    }
}

#[test]
fn undo_tree_mouse_scrolls_hovered_panes_and_selects_without_restoring() {
    use app::state::UndoTreeSurfaceRole;

    let files = MouseFiles::new();
    let document = files.file("history.txt", &"original line\n".repeat(80));
    let mut state = EditorState::new(EditorSession::open_initial_file(&document).unwrap());
    state.configure_mouse(true, false, false);
    let mut window = TestWindow::new(120, 36);
    render(&mut state, &mut window);
    for number in 0..40 {
        state.apply_input(InputAction::Paste(format!("edit {number}\n")), 120, 36);
    }
    let preview_line = format!("ab界e\u{301} latest change {}", "abcdef".repeat(15));
    state.apply_input(
        InputAction::Paste(format!("{preview_line}\n").repeat(40)),
        120,
        36,
    );
    let source_id = state.session.active_id();
    let source_text = state.session.active_buffer().to_string();
    state.apply_input(InputAction::ToggleUndoTree, 120, 36);
    render(&mut state, &mut window);

    let tree_pane = state
        .panes()
        .iter()
        .find(|pane| {
            state.undo_tree_surface_role(pane.buffer_id) == Some(UndoTreeSurfaceRole::Tree)
        })
        .unwrap()
        .clone();
    let preview_pane = state
        .panes()
        .iter()
        .find(|pane| {
            state.undo_tree_surface_role(pane.buffer_id) == Some(UndoTreeSurfaceRole::Preview)
        })
        .unwrap()
        .clone();
    let source_pane = state
        .panes()
        .iter()
        .find(|pane| pane.buffer_id == source_id)
        .unwrap()
        .clone();
    let rectangles = state.pane_rects(120, 35);
    let tree = rectangles
        .iter()
        .find(|rect| rect.pane_id == tree_pane.id)
        .unwrap();
    let preview = rectangles
        .iter()
        .find(|rect| rect.pane_id == preview_pane.id)
        .unwrap();
    let source = rectangles
        .iter()
        .find(|rect| rect.pane_id == source_pane.id)
        .unwrap();
    let wheel = |state: &mut EditorState, window: &mut TestWindow, x, y| {
        handle_editor_event(state, &mut None, Event::MouseScroll { x, y, delta: 1 });
        render(state, window);
    };
    let pane_scroll = |state: &EditorState, pane_id| {
        state
            .panes()
            .iter()
            .find(|pane| pane.id == pane_id)
            .unwrap()
            .view
            .cursor
            .scroll_y_lines
    };

    wheel(&mut state, &mut window, preview.x + 2, preview.y + 2);
    assert_eq!(pane_scroll(&state, preview_pane.id), 3);
    assert_eq!(state.active_pane_id(), tree_pane.id);

    let horizontal = |state: &mut EditorState, window: &mut TestWindow, delta| {
        handle_editor_event(
            state,
            &mut None,
            Event::MouseScrollHorizontal {
                x: preview.x + 2,
                y: preview.y + 2,
                delta,
            },
        );
        render(state, window);
    };
    for _ in 0..3 {
        horizontal(&mut state, &mut window, 1);
    }
    state.with_buffer_view_mut(preview_pane.buffer_id, |_, view| {
        assert_eq!(view.cursor.viewport_scroll(), (3, 3));
    });
    let visible: String = window.cells[(preview.y + 1) as usize]
        [preview.x as usize..preview.x as usize + 10]
        .iter()
        .collect();
    assert_eq!(visible, " e latest ");
    assert_eq!(state.active_pane_id(), tree_pane.id);
    assert_eq!(state.active_cursor_pos(), Pos::zero());

    state.configure_mouse(true, false, true);
    horizontal(&mut state, &mut window, 1);
    state.with_buffer_view_mut(preview_pane.buffer_id, |_, view| {
        assert_eq!(view.cursor.viewport_scroll(), (0, 3));
    });
    state.configure_mouse(true, false, false);
    horizontal(&mut state, &mut window, 127);
    let max_scroll = minui::cell_width(&preview_line, minui::TabPolicy::Fixed(4)) as usize
        - preview.width as usize;
    state.with_buffer_view_mut(preview_pane.buffer_id, |_, view| {
        assert_eq!(view.cursor.viewport_scroll(), (max_scroll, 3));
    });
    assert_eq!(
        window.cells[(preview.y + 1) as usize][(preview.x + preview.width - 1) as usize],
        'f'
    );
    horizontal(&mut state, &mut window, -127);
    state.with_buffer_view_mut(preview_pane.buffer_id, |_, view| {
        assert_eq!(view.cursor.viewport_scroll(), (0, 3));
    });
    horizontal(&mut state, &mut window, 1);
    click(&mut state, &mut window, preview.x + 2, preview.y + 2);

    wheel(&mut state, &mut window, tree.x + 2, tree.y + 2);
    assert_eq!(pane_scroll(&state, tree_pane.id), 3);
    assert_eq!(state.active_cursor_pos(), Pos::new(3, 0));
    assert_eq!(pane_scroll(&state, preview_pane.id), 0);
    state.with_buffer_view_mut(preview_pane.buffer_id, |_, view| {
        assert_eq!(view.cursor.scroll_x_cells, 0);
    });
    assert!(
        state
            .session
            .buffer(preview_pane.buffer_id)
            .unwrap()
            .to_string()
            .starts_with("Node: 38\n")
    );

    let source_scroll = pane_scroll(&state, source_pane.id);
    wheel(&mut state, &mut window, source.x + 8, source.y + 3);
    assert_eq!(pane_scroll(&state, source_pane.id), source_scroll + 3);
    assert_eq!(state.active_pane_id(), tree_pane.id);
    let clicked_line = state
        .session
        .buffer(source_id)
        .unwrap()
        .line_string(source_scroll + 5);
    let source_row: String = window.cells[(source.y + 3) as usize][source.x as usize..]
        .iter()
        .collect();
    let content_x = source_row[..source_row.find(&clicked_line).unwrap()]
        .chars()
        .count() as u16;
    click(
        &mut state,
        &mut window,
        source.x + content_x + 2,
        source.y + 3,
    );
    assert_eq!(state.active_pane_id(), source_pane.id);
    assert_eq!(state.active_cursor_pos(), Pos::new(source_scroll + 5, 2));
    let source_cursor = state.active_cursor_pos();

    wheel(&mut state, &mut window, tree.x + 2, tree.y + 2);
    assert_eq!(pane_scroll(&state, tree_pane.id), 6);
    assert_eq!(state.active_pane_id(), source_pane.id);
    assert_eq!(state.active_cursor_pos(), source_cursor);
    click(&mut state, &mut window, tree.x + 2, tree.y + 3);
    assert_eq!(state.active_pane_id(), tree_pane.id);
    assert_eq!(state.active_cursor_pos(), Pos::new(8, 0));
    assert!(
        state
            .session
            .buffer(preview_pane.buffer_id)
            .unwrap()
            .to_string()
            .starts_with("Node: 33\n")
    );
    click(&mut state, &mut window, preview.x + 2, preview.y + 2);
    assert_eq!(state.active_pane_id(), tree_pane.id);
    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(
        state.session.buffer(source_id).unwrap().to_string(),
        source_text
    );
}

#[test]
fn explorer_mouse_opens_entries_and_dismisses_only_the_top_layer() {
    let files = MouseFiles::new();
    let background = files.file("background.txt", "editor text\n");
    let nested = files.file("folder/nested.txt", "nested text\n");
    let mut state = EditorState::new(EditorSession::open_initial_file(&background).unwrap());
    state.configure_mouse(true, false, false);
    state
        .open_explorer_at_path(background.parent().unwrap().to_path_buf())
        .unwrap();
    let mut window = TestWindow::new(100, 30);
    render(&mut state, &mut window);
    let directory = explorer_entry(&state, "folder/");
    click(&mut state, &mut window, directory.x, directory.y);
    assert_eq!(
        state.explorer_popup().unwrap().dir_path,
        background.parent().unwrap()
    );
    click(&mut state, &mut window, directory.x, directory.y);
    assert_eq!(
        state.explorer_popup().unwrap().dir_path,
        nested.parent().unwrap()
    );

    state.apply_input(InputAction::EnterCommand, 100, 30);
    render(&mut state, &mut window);
    let explorer_cursor = state.active_cursor_pos();
    click(&mut state, &mut window, 0, 0);
    assert_eq!(state.mode, EditorMode::Normal);
    assert!(
        state.explorer_popup().is_some(),
        "only the command popup should close"
    );
    assert_eq!(state.active_cursor_pos(), explorer_cursor);

    let entry = explorer_entry(&state, "nested.txt");
    click(&mut state, &mut window, entry.x, entry.y);
    assert!(state.explorer_popup().is_some());
    click(&mut state, &mut window, entry.x, entry.y);
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(nested.as_path())
    );
    assert!(state.explorer_popup().is_none());

    state
        .open_explorer_at_path(nested.parent().unwrap().to_path_buf())
        .unwrap();
    state
        .session
        .active_buffer_mut()
        .insert(Pos::zero(), "edited");
    state.session.recompute_active_dirty();
    let draft = state.session.active_buffer().to_string();
    render(&mut state, &mut window);
    click(&mut state, &mut window, 0, 0);
    assert!(
        state.explorer_popup().is_some(),
        "outside clicks must preserve unsaved explorer edits"
    );
    assert_eq!(state.session.active_buffer().to_string(), draft);

    state.apply_input(InputAction::SurfaceGoParent, 100, 30);
    render(&mut state, &mut window);
    click(&mut state, &mut window, 0, 0);
    assert!(
        state.explorer_popup().is_some(),
        "outside clicks must also preserve drafts in another directory"
    );
    let directory = explorer_entry(&state, "folder/");
    click(&mut state, &mut window, directory.x, directory.y);
    click(&mut state, &mut window, directory.x, directory.y);
    assert_eq!(state.session.active_buffer().to_string(), draft);
}

#[test]
fn pinboard_double_click_opens_occupied_slots_without_assigning_empty_ones() {
    let files = MouseFiles::new();
    let alpha = files.file("alpha.txt", "alpha\n");
    let beta = files.file("beta.txt", "beta\n");
    let mut state = EditorState::new(EditorSession::open_initial_file(&alpha).unwrap());
    state.configure_mouse(true, false, false);
    state.apply_input(InputAction::QuickPinCurrentFile, 100, 30);
    state.apply_input(InputAction::AssignPinSlot { slot: 2 }, 100, 30);
    state.session.open_file(&beta).unwrap();
    state.apply_input(InputAction::QuickPinCurrentFile, 100, 30);
    let slots = state.pin_slots_for_test();
    let storage = files.root.join("state/redox/pinned-files.json");
    let saved = fs::read(&storage).unwrap();
    let mut window = TestWindow::new(100, 30);
    render(&mut state, &mut window);
    let empty = target_rect(
        &state,
        MousePopup::Pinboard,
        &MouseTarget::Entry {
            index: 0,
            identity: String::new(),
        },
    );
    click(&mut state, &mut window, empty.x, empty.y);
    click(&mut state, &mut window, empty.x, empty.y);
    assert_eq!(state.mode, EditorMode::PinSelect);
    assert_eq!(state.pin_slots_for_test(), slots);
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(beta.as_path())
    );

    let occupied = target_rect(
        &state,
        MousePopup::Pinboard,
        &MouseTarget::Entry {
            index: 2,
            identity: "alpha.txt".to_string(),
        },
    );
    click(&mut state, &mut window, occupied.x, occupied.y);
    assert_eq!(state.pin_selector_popup().unwrap().selected, 2);
    click(&mut state, &mut window, occupied.x, occupied.y);
    assert_eq!(state.mode, EditorMode::Normal);
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(alpha.as_path())
    );
    assert_eq!(state.pin_slots_for_test(), slots);
    assert_eq!(fs::read(storage).unwrap(), saved);
}

#[test]
fn nested_split_mouse_switches_independent_views_of_the_same_buffer() {
    let _lock = app::state::global_test_state_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() =
        TextBuffer::from_text(&format!("{}\n", "abcdef".repeat(40)).repeat(120));
    state.configure_mouse(true, false, false);
    state.zen.enabled = true;
    state.zen.hide_gutter = false;
    state.zen.width_percent = 80;
    state.zen.min_width = 1;
    let mut window = TestWindow::new(120, 40);
    render(&mut state, &mut window);
    state.split_active_pane(app::state::SplitAxis::Vertical);
    state.split_active_pane(app::state::SplitAxis::Horizontal);
    render(&mut state, &mut window);
    let panes = state.pane_rects(96, 39);
    let buffer = state.session.active_id();
    for (index, pane) in panes.iter().enumerate() {
        let content = pane_content_x(&state, pane.pane_id);
        click(
            &mut state,
            &mut window,
            12 + pane.x + content + 3,
            pane.y + 3,
        );
        assert_eq!(state.active_pane_id(), pane.pane_id);
        state.with_active_buffer_view_mut(|_, view| {
            view.cursor.scroll_x_cells = index * 8;
            view.cursor.scroll_y_lines = index * 20;
            view.cursor
                .place_cursor(Pos::new(index * 20 + 6, index * 8 + 3));
        });
        state.sync_active_pane_view();
        render(&mut state, &mut window);
    }
    for pane in &panes {
        let view = &state
            .panes()
            .iter()
            .find(|candidate| candidate.id == pane.pane_id)
            .unwrap()
            .view;
        let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
        let header = u16::from(pane.pane_id != state.active_pane_id());
        let content = pane_content_x(&state, pane.pane_id);
        click(
            &mut state,
            &mut window,
            12 + pane.x + content + 3,
            pane.y + header + 2,
        );
        assert_eq!(state.active_pane_id(), pane.pane_id);
        assert_eq!(state.session.active_id(), buffer);
        assert_eq!(
            state.active_cursor_pos(),
            Pos::new(scroll_y + 2, scroll_x + 3)
        );
        state.with_active_buffer_view_mut(|_, view| {
            assert_eq!(view.cursor.viewport_scroll(), (scroll_x, scroll_y))
        });
    }
    let active = state.active_pane_id();
    for hovered in &panes {
        let before = state.panes().to_vec();
        handle_editor_event(
            &mut state,
            &mut None,
            Event::MouseScroll {
                x: 12 + hovered.x + 8,
                y: hovered.y + 3,
                delta: 1,
            },
        );
        render(&mut state, &mut window);
        assert_eq!(state.active_pane_id(), active);
        for (previous, current) in before.iter().zip(state.panes()) {
            let expected = previous.view.cursor.scroll_y_lines
                + if current.id == hovered.pane_id { 3 } else { 0 };
            assert_eq!(current.view.cursor.scroll_y_lines, expected);
            assert_eq!(
                current.view.cursor.scroll_x_cells,
                previous.view.cursor.scroll_x_cells
            );
        }
    }
    let cursor = state.active_cursor_pos();
    for (x, y) in [(0, 3), (119, 3), (20, 39)] {
        click(&mut state, &mut window, x, y);
        assert_eq!(state.active_pane_id(), active);
        assert_eq!(state.active_cursor_pos(), cursor);
    }
}
