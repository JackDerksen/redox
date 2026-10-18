mod mouse;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use minui::KeybindAction;
use minui::input::KeyboardHandler;
use minui::prelude::{
    input::{Event, KeyKind},
    render::{Color, Window},
};
use redox_core::{EditorSession, TextBuffer};

use super::*;
use crate::app::{EditorState, FramePerfSample};
use crate::event_loop::{
    apply_runtime_colorscheme, handle_editor_event, open_runtime_config, reload_runtime_config,
};
use crate::input::{InputAction, map_event_with_context};
use crate::launch::LaunchConfig;
use crate::ui::overlays::{HighlightFade, LineDecorations, draw_delimiter_highlights};
use crate::ui::render::LineViewport;
use crate::ui::syntax::{draw_line_with_syntax, lexical_fallback_line_spans};
use crate::ui::text_style::TextStyle;
use crate::ui::widgets::dashboard::draw_dashboard;
use crate::ui::widgets::pane::draw_pane_filename;
use crate::ui::widgets::popup::anchored_popup_origin;
use crate::ui::{
    TextViewport, UiStyle, draw_command_line_popup, explorer_popup_inner_size, language_for_path,
};
use crate::view::buffer::{SnapshotOverlays, SnapshotStyle, SnapshotViewport, draw_snapshot_lines};
use crate::view::cells::{
    search_highlight_cells, selected_visible_cells, visual_selection_visible_cells,
};
use crate::view::draw_buffer_view;
use crate::view::gutter::{draw_line_numbers, line_number_gutter_width, pane_content_x};
use crate::view::line::{
    LineHighlights, draw_line_with_highlights, draw_plain_line, draw_snippet_placeholders,
};
use crate::view::panes::inactive_pane_filename;
use crate::view::popups::popup_stack_layout;

#[test]
fn mouse_scroll_and_selection_follow_the_rendered_viewport() {
    use minui::MouseButton;
    use redox_core::Pos;
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(&"\t界e\u{301}👩🏽‍💻 tail\n".repeat(40));
    let mut window = TestWindow::new(60, 12);
    let mut perf = FramePerfSample::default();
    let style = UiStyle::default();
    let mut clipboard = None;
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    let content = pane_content_x(&state, state.active_pane_id());
    let click = |x, y, button| Event::MouseClick { x, y, button };
    handle_editor_event(
        &mut state,
        &mut clipboard,
        click(content + 5, 1, MouseButton::Left),
    );
    assert_eq!(
        state.active_cursor_pos(),
        Pos::zero(),
        "ignored before mouse configuration"
    );
    let (config, _) = LaunchConfig {
        default_only: true,
        ..LaunchConfig::default()
    }
    .load()
    .unwrap();
    state.configure_mouse(
        config.mouse,
        config.mouse_invert_vertical,
        config.mouse_invert_horizontal,
        config.mouse_scroll_step_vertical,
        config.mouse_scroll_step_horizontal,
    );
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseScroll {
            x: 8,
            y: 0,
            delta: 1,
        },
    );
    for _ in 0..3 {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::MouseScrollHorizontal {
                x: 8,
                y: 0,
                delta: 1,
            },
        );
    }
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    state.with_active_buffer_view_mut(|buffer, view| {
        assert_eq!(view.cursor.viewport_scroll(), (3, 3));
        assert_eq!(view.cursor.cursor, Pos::zero());
        assert!(!view.cursor.cursor_spec(buffer, 55, 11).visible);
    });
    handle_editor_event(
        &mut state,
        &mut clipboard,
        click(content + 2, 1, MouseButton::Left),
    );
    assert_eq!(
        state.active_cursor_pos(),
        Pos::new(4, 1),
        "wide character after a clipped tab"
    );
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseDrag {
            x: content + 4,
            y: 1,
            button: MouseButton::Left,
        },
    );
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseRelease {
            x: content + 4,
            y: 1,
            button: MouseButton::Left,
        },
    );
    assert_eq!(state.mode, crate::app::EditorMode::Visual);
    let (selection, _) = state.active_visual_selection().unwrap();
    assert_eq!(
        state
            .session
            .active_buffer()
            .visual_charwise_text(selection),
        "界e\u{301}👩🏽‍💻"
    );
    handle_editor_event(
        &mut state,
        &mut clipboard,
        click(59, 10, MouseButton::Right),
    );
    assert_eq!(state.active_visual_selection().unwrap().0, selection);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert_eq!(state.active_visual_selection().unwrap().0, selection);
    state.configure_mouse(false, false, false, 3, 3);
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseScroll {
            x: 8,
            y: 0,
            delta: 1,
        },
    );
    state.with_active_buffer_view_mut(|_, view| assert_eq!(view.cursor.viewport_scroll(), (3, 3)));
    state.configure_mouse(true, true, true, 1, 2);
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseScroll {
            x: 8,
            y: 0,
            delta: 1,
        },
    );
    for _ in 0..3 {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::MouseScrollHorizontal {
                x: 8,
                y: 0,
                delta: 1,
            },
        );
    }
    state.with_active_buffer_view_mut(|_, view| assert_eq!(view.cursor.viewport_scroll(), (1, 2)));
}

#[test]
fn offscreen_cursor_does_not_linger_while_the_page_scrolls() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    for split in [false, true] {
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        *state.session.active_buffer_mut() = TextBuffer::from_text(&"abcdef\n".repeat(40));
        state.zen.enabled = split;
        state.zen.hide_gutter = false;
        state.zen.width_percent = 80;
        state.zen.min_width = 1;
        if split {
            state.split_active_pane(crate::app::state::SplitAxis::Vertical);
        }
        state.with_active_buffer_view_mut(|_, view| {
            view.cursor.place_cursor(redox_core::Pos::new(10, 0));
        });
        let mut window = TestWindow::new(80, 24);
        let mut perf = FramePerfSample::default();
        let style = UiStyle::default();
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert!(window.cursor.unwrap().visible);

        // Keep the same terminal cursor state across frames, as MinUI does.
        for scroll_y in [0, 3, 6, 2] {
            state.with_active_buffer_view_mut(|_, view| {
                view.cursor.scroll_x_cells = 3;
                view.cursor.scroll_y_lines = scroll_y;
            });
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            assert!(
                !window.cursor.unwrap().visible,
                "split={split}, top={scroll_y}"
            );
        }
        state.with_active_buffer_view_mut(|_, view| view.cursor.scroll_x_cells = 0);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        let cursor = window.cursor.unwrap();
        assert!(cursor.visible);
        assert_eq!(cursor.y, 8);
        let gutter_end = usize::from(cursor.x);
        assert_eq!(
            window.cells[usize::from(cursor.y)][gutter_end - 4..gutter_end]
                .iter()
                .filter(|character| character.is_ascii_digit())
                .collect::<String>(),
            "11",
            "cursor and current line number must agree, split={split}"
        );
    }
}

#[test]
fn mouse_clicks_account_for_zen_margins_and_inactive_pane_overlays() {
    use minui::MouseButton;
    use redox_core::Pos;
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(&"abcdef\n".repeat(40));
    state.configure_mouse(true, false, false, 3, 3);
    state.zen.enabled = true;
    state.zen.width_percent = 80;
    state.zen.min_width = 1;
    let mut window = TestWindow::new(100, 12);
    let mut perf = FramePerfSample::default();
    let style = UiStyle::default();
    let mut clipboard = None;
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    state.split_active_pane(crate::app::state::SplitAxis::Vertical);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    let inactive = state
        .pane_rects(80, 11)
        .into_iter()
        .find(|rect| rect.pane_id != state.active_pane_id())
        .unwrap();
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 10 + inactive.x + 3,
            y: inactive.y + 2,
            button: MouseButton::Left,
        },
    );
    assert_eq!(state.active_pane_id(), inactive.pane_id);
    assert_eq!(state.active_cursor_pos(), Pos::new(2, 3));
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    for (x, y) in [(0, 2), (99, 2), (15, 11)] {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::MouseClick {
                x,
                y,
                button: MouseButton::Left,
            },
        );
        assert_eq!(state.active_cursor_pos(), Pos::new(2, 3));
    }
    state.mode = crate::app::EditorMode::Insert;
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::KeyWithModifiers(minui::KeyWithModifiers {
            key: KeyKind::Right,
            mods: minui::KeyModifiers::ctrl(),
        }),
    );
    assert_eq!(state.pane_rects(80, 11)[0].width, inactive.width + 1);
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 10 + inactive.width + 1,
            y: 2,
            button: MouseButton::Left,
        },
    );
    for x in [60, 65] {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::MouseDrag {
                x,
                y: 2,
                button: MouseButton::Left,
            },
        );
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert_eq!(state.pane_rects(80, 11)[0].width, x - 10);
    }
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseRelease {
            x: 65,
            y: 2,
            button: MouseButton::Left,
        },
    );
    assert_eq!(state.mode, crate::app::EditorMode::Insert);
    assert_eq!(state.active_cursor_pos(), Pos::new(2, 3));
    state.configure_mouse(false, false, false, 3, 3);
    for event in [
        Event::MouseClick {
            x: 65,
            y: 2,
            button: MouseButton::Left,
        },
        Event::MouseDrag {
            x: 75,
            y: 2,
            button: MouseButton::Left,
        },
    ] {
        handle_editor_event(&mut state, &mut clipboard, event);
    }
    assert_eq!(state.pane_rects(80, 11)[0].width, 55);
    state.configure_mouse(true, false, false, 3, 3);
    state.mode = crate::app::EditorMode::Normal;
    handle_editor_event(&mut state, &mut clipboard, Event::Character(':'));
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 15,
            y: 4,
            button: MouseButton::Left,
        },
    );
    assert_eq!(state.mode, crate::app::EditorMode::Command);
    assert_eq!(state.active_cursor_pos(), Pos::new(2, 3));
}

#[test]
fn rain_keeps_the_active_pane_colours_after_the_command_prompt_closes() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text("rain on me\n");
    let style = UiStyle::default();
    let mut window = TestWindow::new(40, 10);
    let mut perf = FramePerfSample::default();
    let mut clipboard = None;
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    let content_x = pane_content_x(&state, state.active_pane_id()) as usize;
    let resting = window.foregrounds[0][content_x];

    // The prompt dims the editor; its fade back out is still under way
    // when the first rain frame captures the text.
    for character in ":rain".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    std::thread::sleep(
        Duration::from_millis(state.animations.focus_fade_ms) + Duration::from_millis(50),
    );
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert!(state.active_rain_animation().is_some());
    assert_eq!(window.foregrounds[0][content_x], resting);
}

#[test]
fn command_prompt_is_visible_and_executable_after_rain_and_modal_input() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    for context in [
        "insert",
        "search",
        "finder",
        "pins",
        "dashboard",
        "rain",
        "lsp",
        "about",
        "perf",
        "explorer",
        "prefix",
    ] {
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        let mut clipboard = None;
        for character in "iabc".chars() {
            handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        if context != "insert" {
            handle_editor_event(&mut state, &mut clipboard, Event::Escape);
            match context {
                "search" => state.apply_input(InputAction::EnterSearch, 80, 24),
                "finder" | "pins" => {
                    state.apply_input(InputAction::OpenFinder, 80, 24);
                    if context == "pins" {
                        state.apply_input(InputAction::FinderBeginPin, 80, 24);
                    }
                }
                "prefix" => {
                    handle_editor_event(&mut state, &mut clipboard, Event::Character(' '));
                }
                command => {
                    state.apply_input(InputAction::RunCommand(command.to_string()), 80, 24);
                }
            }
        }
        if matches!(
            context,
            "insert" | "search" | "finder" | "pins" | "lsp" | "prefix"
        ) {
            handle_editor_event(&mut state, &mut clipboard, Event::Escape);
            if context == "pins" {
                handle_editor_event(&mut state, &mut clipboard, Event::Escape);
            }
        }
        if context == "rain" {
            assert!(state.rain_is_active());
            handle_editor_event(&mut state, &mut clipboard, Event::Character('i'));
            assert!(state.rain_is_active());
            assert_eq!(state.mode, crate::app::EditorMode::Normal);
        }
        assert!(handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Character(':')
        ));
        assert_eq!(state.mode, crate::app::EditorMode::Command, "{context}");
        assert!(!state.rain_is_active(), "{context}");
        assert!(state.finder_popup().is_none(), "{context}");
        assert!(state.pin_selector_popup().is_none(), "{context}");
        assert!(state.lsp_marketplace_popup().is_none(), "{context}");

        if context == "rain" {
            handle_editor_event(&mut state, &mut clipboard, Event::Escape);
            assert_eq!(state.mode, crate::app::EditorMode::Normal);
            for character in ":rain".chars() {
                handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
            }
            handle_editor_event(&mut state, &mut clipboard, Event::Enter);
            assert!(state.rain_is_active());
            handle_editor_event(&mut state, &mut clipboard, Event::Escape);
            assert!(!state.rain_is_active());
            assert_eq!(state.mode, crate::app::EditorMode::Normal);
            state.apply_input(InputAction::RunCommand("rain".to_string()), 80, 24);
            let shifted_colon = Event::KeyWithModifiers(minui::prelude::input::KeyWithModifiers {
                key: KeyKind::Char(':'),
                mods: minui::prelude::input::KeyModifiers::shift(),
            });
            handle_editor_event(&mut state, &mut clipboard, shifted_colon);
            assert_eq!(state.mode, crate::app::EditorMode::Command);
            assert!(!state.rain_is_active());
        }
        for character in "q!".chars() {
            handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        let mut window = TestWindow::new(80, 24);
        draw_buffer_view(
            &mut state,
            &UiStyle::default(),
            &mut window,
            &mut FramePerfSample::default(),
        )
        .unwrap();
        assert!(
            (0..24).any(|row| window.row_text(row).contains("q!")),
            "command line hidden in {context}"
        );
        assert!(window.cursor.is_some(), "{context}");
        assert!(!handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Enter
        ));
        assert!(state.should_quit, "{context}");
    }
}

#[test]
fn dashboard_navigation_and_shortcuts_reach_the_existing_editor_flows() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    let mut clipboard = None;
    state.open_dashboard();
    assert!(state.about_popup().is_none());
    assert_eq!(state.dashboard_selection(), Some(0));
    let style = UiStyle::default();
    let mut perf = FramePerfSample::default();
    for (icons_enabled, expected_menu_width, expected_icon_prefix_width) in
        [(false, 30u16, 0), (true, 33, 3)]
    {
        let style = UiStyle {
            icons_enabled,
            ..style
        };
        for (width, height, shows_full_menu) in [
            (120, 40, true),
            (80, 24, true),
            (32, 14, true),
            (10, 4, false),
            (1, 1, false),
            (0, 0, false),
        ] {
            let mut window = TestWindow::new(width, height);
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            if width > 0 && height > 0 {
                let cursor = window.cursor.expect("dashboard cursor");
                let menu_width = expected_menu_width.min(width);
                let menu_left = (width - menu_width) / 2;
                assert_eq!(cursor.x, menu_left + menu_width - 1);
                assert_eq!(window.cells[cursor.y as usize][cursor.x as usize], 'r');
                if shows_full_menu {
                    let row = &window.cells[cursor.y as usize];
                    let label_column = (menu_left + expected_icon_prefix_width) as usize;
                    assert_eq!(row[label_column], 'R');
                    if icons_enabled {
                        assert_eq!(
                            row[menu_left as usize],
                            crate::ui::icons::UNDO_TREE.chars().next().unwrap()
                        );
                        assert!(
                            row[menu_left as usize + 1..label_column]
                                .iter()
                                .all(|cell| *cell == ' ')
                        );
                    }
                    let screen = window
                        .cells
                        .iter()
                        .map(|row| row.iter().collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(!screen.contains("j/k to move"));
                    for (hotkey, label) in crate::app::state::dashboard::DASHBOARD_ITEMS {
                        let row = window
                            .cells
                            .iter()
                            .find(|row| row.iter().collect::<String>().contains(label))
                            .unwrap_or_else(|| {
                                panic!("missing {label} at {width}x{height}: {screen}")
                            });
                        assert_eq!(row[cursor.x as usize], hotkey);
                    }
                }
            }
        }
    }
    for hotkey in ['f', 'e'] {
        assert!(handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Character(hotkey)
        ));
        if hotkey == 'f' {
            assert_eq!(state.mode, crate::app::EditorMode::Finder);
        } else {
            assert!(state.explorer_popup().is_some());
        }
        let mut popup_window = TestWindow::new(120, 40);
        draw_buffer_view(&mut state, &style, &mut popup_window, &mut perf).unwrap();
        assert!(
            popup_window
                .row_text(popup_window.get_size().1 - 1)
                .trim()
                .is_empty()
        );
        assert!(handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Escape
        ));
        assert_eq!(state.mode, crate::app::EditorMode::Normal);
        assert_eq!(state.dashboard_selection(), Some(0));
    }
    for hotkey in "jjjkj".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(hotkey));
    }
    assert_eq!(state.dashboard_selection(), Some(3));
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    assert!(state.dashboard_selection().is_none());
    assert!(state.session.active_meta().path.is_none());
    assert!(state.session.active_buffer().is_empty());
    assert_eq!(state.session.summaries().len(), 1);

    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("new file.rs");
    for character in "ifn main() {}   ".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    for character in format!(":w {}", destination.display()).chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    assert_eq!(fs::read_to_string(&destination).unwrap(), "fn main() {}\n");
    assert_eq!(
        language_for_path(state.session.active_meta().path.as_deref()),
        Some(crate::ui::syntax::SyntaxLanguage::Rust)
    );
    assert!(!state.session.active_meta().dirty);

    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    state.open_dashboard();
    handle_editor_event(&mut state, &mut clipboard, Event::Character('c'));
    assert!(state.take_config_open_request());
    state.open_config_file(&directory.path().join("config.toml"));
    assert!(state.dashboard_selection().is_none());
    assert_eq!(state.session.summaries().len(), 1);

    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    state.open_dashboard();
    assert!(!handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::Character('q')
    ));
}

#[test]
fn dashboard_command_preserves_edits_and_panes_and_hides_only_its_statusline() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
    fs::write(&path, "saved\n").unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
    let original_id = state.session.active_id();
    state
        .session
        .active_buffer_mut()
        .insert(redox_core::Pos::zero(), "unsaved ");
    state.session.recompute_active_dirty();
    state.split_active_pane(crate::app::state::SplitAxis::Vertical);
    state.with_active_buffer_view_mut(|_, view| view.cursor.cursor = redox_core::Pos::new(0, 4));
    let original_panes = state
        .panes()
        .iter()
        .map(|pane| pane.buffer_id)
        .collect::<Vec<_>>();
    let mut clipboard = None;
    let style = UiStyle::default();
    let mut perf = FramePerfSample::default();

    for character in ":dash".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    state.apply_input(InputAction::CommandComplete, 120, 40);
    assert_eq!(state.command_line, "dashboard");
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    assert_eq!(state.dashboard_selection(), Some(0));
    state.apply_input(InputAction::RunCommand("dashboard".to_string()), 120, 40);
    assert_eq!(
        state.session.summaries().len(),
        2,
        "reuse the dashboard buffer"
    );

    for hotkey in ['f', 'e'] {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(hotkey));
        let mut window = TestWindow::new(120, 40);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert!(window.row_text(window.get_size().1 - 1).trim().is_empty());
        assert!(
            !window
                .cells
                .iter()
                .any(|row| row.iter().collect::<String>().contains("unsaved saved"))
        );
        handle_editor_event(&mut state, &mut clipboard, Event::Escape);
        assert_eq!(state.dashboard_selection(), Some(0));
    }
    assert_eq!(
        state
            .panes()
            .iter()
            .map(|pane| pane.buffer_id)
            .collect::<Vec<_>>(),
        original_panes
    );
    assert!(handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::Character('q')
    ));
    assert!(state.session.any_dirty());
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    assert!(state.dashboard_selection().is_none());
    assert_eq!(state.session.summaries().len(), 1);
    assert_eq!(state.session.active_id(), original_id);
    assert_eq!(state.session.active_buffer().to_string(), "unsaved saved\n");
    assert_eq!(state.active_cursor_pos(), redox_core::Pos::new(0, 4));
    assert_eq!(fs::read_to_string(&path).unwrap(), "saved\n");

    for action in [
        InputAction::OpenFinder,
        InputAction::RunCommand("explorer".to_string()),
    ] {
        state.apply_input(action, 120, 40);
        let mut window = TestWindow::new(120, 40);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert!(window.row_text(window.get_size().1 - 1).contains("NORMAL"));
        handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    }
    state.apply_input(InputAction::RunCommand("dashboard".to_string()), 120, 40);
    handle_editor_event(&mut state, &mut clipboard, Event::Character('n'));
    assert!(state.dashboard_selection().is_none());
    assert!(state.session.active_meta().path.is_none());
    assert!(state.session.active_buffer().is_empty());
    assert_eq!(
        state.session.buffer(original_id).unwrap().to_string(),
        "unsaved saved\n"
    );
}

#[test]
fn substitution_confirmation_accepts_skips_cancels_and_groups_undo() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let text = "foo foo\nfoo foo\nfoo\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("confirmation.txt");
    std::fs::write(&path, text).unwrap();
    for (command, answers, expected) in [
        (":2,3s/foo/雪/gc", "nya", "foo foo\nfoo 雪\n雪\n"),
        (":s/foo/雪/gc", "yna", "雪 foo\n雪 雪\n雪\n"),
        (":s/foo/雪/gc", "yq", "雪 foo\nfoo foo\nfoo\n"),
        (":s/foo/雪/gc", "yl", "雪 雪\nfoo foo\nfoo\n"),
        (":s/foo/雪/gc", "y\u{1b}", "雪 foo\nfoo foo\nfoo\n"),
        (":s/foo/雪/gc", "q", text),
        (":s/foo/雪/gc", "nnnnn", text),
        (":s/foo/雪/c", "a", "雪 foo\n雪 foo\n雪\n"),
        (":s/foo//gc", "yna", " foo\n \n\n"),
        (r":s/foo/x\ry/gc", "nya", "foo x\ny\nx\ny x\ny\nx\ny\n"),
        (":s/foo/foo/gc", "a", text),
        (":s/^/>/gc", "a", ">foo foo\n>foo foo\n>foo\n"),
        (r":s/foo\nfoo/雪/gc", "a", "foo 雪 雪\n"),
    ] {
        let mut state = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
        state.set_viewport_size(100, 30);
        let mut clipboard = None;
        for character in command.chars() {
            handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        assert!(state.has_substitute_confirmation(), "{command}");
        assert_eq!(state.session.active_buffer().to_string(), text);
        assert!(state.substitute_preview().unwrap().buffer.is_none());
        assert_eq!(state.substitute_preview().unwrap().match_count(), 1);
        handle_editor_event(&mut state, &mut clipboard, Event::Character('x'));
        handle_editor_event(&mut state, &mut clipboard, Event::Paste("ignored".into()));
        assert_eq!(state.command_line, &command[1..]);
        let mut window = TestWindow::new(100, 30);
        draw_buffer_view(
            &mut state,
            &UiStyle::default(),
            &mut window,
            &mut FramePerfSample::default(),
        )
        .unwrap();
        assert!((0..30).any(|row| window.row_text(row).contains("Confirm substitution")));
        assert!((0..30).any(|row| window.row_text(row).contains("q/Esc=quit")));
        let cursor = state.active_cursor_pos();
        let row = cursor.line
            - state.views[&state.session.active_id()]
                .cursor
                .scroll_y_lines;
        let line = window.row_text(row as u16);
        let column = line[..line.find("foo").unwrap()].chars().count() + cursor.col;
        assert_eq!(
            window.backgrounds[row][column],
            Some(UiStyle::default().substitute_colors(true).bg)
        );
        for (index, character) in answers.chars().enumerate() {
            let event = if character == '\u{1b}' {
                Event::Escape
            } else {
                Event::Character(character)
            };
            handle_editor_event(&mut state, &mut clipboard, event);
            if index == 0 && character == 'y' {
                assert_ne!(state.session.active_buffer().to_string(), text);
            }
        }
        assert!(!state.has_substitute_confirmation(), "{command}: {answers}");
        assert_eq!(state.mode, crate::app::EditorMode::Normal);
        assert_eq!(
            state.session.active_buffer().to_string(),
            expected,
            "{command}: {answers}"
        );
        assert_eq!(state.session.active_meta().dirty, expected != text);
        if expected != text {
            handle_editor_event(&mut state, &mut clipboard, Event::Character('u'));
            assert_eq!(
                state.session.active_buffer().to_string(),
                text,
                "undo {answers}"
            );
            state.apply_input(InputAction::Redo, 100, 30);
            assert_eq!(
                state.session.active_buffer().to_string(),
                expected,
                "redo {answers}"
            );
        }
    }
}

#[test]
fn substitution_preview_renders_in_buffer_without_editing() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text("foo foo\nkeep\n");
    let mut clipboard = None;
    let style = UiStyle::default();
    let mut perf = FramePerfSample::default();
    for character in ":s/foo".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    assert_eq!(state.substitute_preview().unwrap().match_count(), 2);
    for (characters, expected_line, highlighted) in [
        (
            "",
            "foo foo",
            &[true, true, true, false, true, true, true][..],
        ),
        ("/B", "B foo", &[true, false, false, false, false][..]),
        (
            "AR/g",
            "BAR BAR",
            &[true, true, true, false, true, true, true][..],
        ),
    ] {
        for character in characters.chars() {
            handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        let mut window = TestWindow::new(100, 30);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        let row = window.row_text(0);
        let column = row[..row.find(expected_line).expect(expected_line)]
            .chars()
            .count();
        let match_colors = style.substitute_colors(!characters.is_empty());
        for (offset, selected) in highlighted.iter().enumerate().take(expected_line.len()) {
            assert_eq!(
                window.backgrounds[0][column + offset],
                Some(if *selected {
                    match_colors.bg
                } else {
                    style.theme.bg
                }),
                "{expected_line}, column {offset}",
            );
        }
        for (offset, selected) in highlighted.iter().enumerate() {
            if *selected {
                assert_eq!(
                    window.foregrounds[0][column + offset],
                    Some(match_colors.fg)
                );
            }
        }
        assert!(window.row_text(1).contains("keep"));
        assert!(!(0..window.height).any(|row| window.row_text(row).contains("- 1  foo")));
        assert!(
            window
                .cursor
                .is_some_and(|cursor| cursor.y < window.height && cursor.x < window.width)
        );
        assert_eq!(state.session.active_buffer().to_string(), "foo foo\nkeep\n");
        assert!(!state.session.active_meta().dirty);
    }
    for (width, height) in [(32, 14), (10, 4), (1, 1), (0, 0)] {
        let mut window = TestWindow::new(width, height);
        draw_command_line_popup(&state, &style, &mut window).unwrap();
        assert!(
            window
                .cursor
                .is_none_or(|cursor| cursor.x < width && cursor.y < height)
        );
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    let mut window = TestWindow::new(100, 30);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert!(window.row_text(0).contains("foo foo"));
    assert!(state.substitute_preview().is_none());
    assert_eq!(state.session.active_buffer().to_string(), "foo foo\nkeep\n");

    for (text, command, expected_lines) in [
        (
            "foo\nkeep\n",
            r":s/foo/one\rtwo/g",
            vec!["one", "two", "keep"],
        ),
        (
            "foo\nfoo\nkeep\n",
            r":s/foo\nfoo/joined/g",
            vec!["joined", "keep"],
        ),
        ("foo foo\nkeep\n", ":s/foo//g", vec![" ", "keep"]),
    ] {
        *state.session.active_buffer_mut() = TextBuffer::from_text(text);
        for character in command.chars() {
            handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        let mut window = TestWindow::new(100, 30);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        for (row, expected) in expected_lines.iter().enumerate() {
            assert!(
                window.row_text(row as u16).contains(expected),
                "{command}, row {row}"
            );
        }
        assert_eq!(state.session.active_buffer().to_string(), text);
        handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        assert_eq!(
            state.session.active_buffer().to_string(),
            expected_lines.join("\n") + "\n"
        );
        handle_editor_event(&mut state, &mut clipboard, Event::Character('u'));
        assert_eq!(state.session.active_buffer().to_string(), text);
    }
}

#[test]
fn pending_substitution_keeps_text_and_delimiter_overlays_stable() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let source = format!(
        "fn main() {{\n    foo();\n}}\n{}",
        "// padding\n".repeat(8192)
    );
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(&source);
    state.session.active_meta_mut().path = Some(PathBuf::from("preview.rs"));
    state.with_active_buffer_view_mut(|buffer, view| {
        view.cursor.cursor = redox_core::Pos::new(0, 10);
        view.syntax_highlighter
            .replace_cache(crate::ui::syntax::SyntaxHighlighter::compute_cache(
                buffer,
                crate::ui::syntax::SyntaxLanguage::Rust,
            ));
        view.delimiter_pair_cache
            .install(crate::ui::overlays::compute_delimiter_analysis(buffer));
    });
    let style = UiStyle::default();
    let mut perf = FramePerfSample::default();
    let mut original = TestWindow::new(100, 30);
    draw_buffer_view(&mut state, &style, &mut original, &mut perf).unwrap();
    let brace_column = original.cells[0]
        .iter()
        .position(|cell| *cell == '{')
        .unwrap();
    assert_eq!(
        original.backgrounds[0][brace_column],
        Some(style.theme.scope)
    );

    state.apply_input(InputAction::EnterCommand, 100, 30);
    for character in r"s/foo/bar\rline".chars() {
        state.apply_input(InputAction::CommandChar(character), 100, 30);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.substitute_preview().unwrap().pending {
        assert!(Instant::now() < deadline, "substitution preview timed out");
        state.update_background(Instant::now());
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut completed = TestWindow::new(100, 30);
    draw_buffer_view(&mut state, &style, &mut completed, &mut perf).unwrap();
    assert!(completed.row_text(1).contains("bar"));
    assert!(completed.row_text(2).contains("line();"));
    assert_eq!(completed.backgrounds[0][brace_column], Some(style.theme.bg));
    let title_row = (0..completed.height)
        .find(|row| completed.row_text(*row).contains("Substitute:"))
        .unwrap();

    // Do not poll between keystrokes: the first result is obsolete before delivery.
    for character in ['!', '?'] {
        state.apply_input(InputAction::CommandChar(character), 100, 30);
        let preview = state.substitute_preview().unwrap();
        assert!(preview.pending);
        assert_eq!(
            preview.display_position(state.session.active_buffer(), redox_core::Pos::new(2, 0)),
            redox_core::Pos::new(3, 0)
        );
        let mut pending = TestWindow::new(100, 30);
        draw_buffer_view(&mut state, &style, &mut pending, &mut perf).unwrap();
        assert_eq!(pending.cells[..4], completed.cells[..4]);
        assert_eq!(pending.backgrounds[..4], completed.backgrounds[..4]);
        assert_eq!(pending.foregrounds[..4], completed.foregrounds[..4]);
        assert_eq!(pending.row_text(title_row), completed.row_text(title_row));
        assert_eq!(state.session.active_buffer().to_string(), source);
    }
    state.apply_input(InputAction::CommandEnter, 100, 30);
    assert_eq!(state.mode, crate::app::EditorMode::Normal);
    assert_eq!(
        state.session.active_buffer().to_string(),
        source.replacen("foo", "bar\nline!?", 1)
    );
}

#[test]
fn split_panes_keep_their_own_cursors_and_share_undo_history() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(&"line\n".repeat(40));
    state.set_editor_area_size(100, 29);
    let style = UiStyle::default();
    let mut window = TestWindow::new(100, 30);
    let mut perf = FramePerfSample::default();
    let mut clipboard = None;
    state.split_active_pane(crate::app::state::SplitAxis::Vertical);
    let first = state
        .panes()
        .iter()
        .find(|pane| pane.id != state.active_pane_id())
        .unwrap()
        .id;
    for character in "5jix".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    let second = state.active_pane_id();
    assert_eq!(state.active_cursor_pos(), redox_core::Pos::new(5, 0));

    state.focus_split(crate::app::state::SplitDirection::Left);
    assert_eq!(state.active_pane_id(), first);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert_eq!(state.active_cursor_pos(), redox_core::Pos::zero());
    handle_editor_event(&mut state, &mut clipboard, Event::Character('u'));
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert_eq!(state.session.active_buffer().line_string(5), "line");

    state.focus_split(crate::app::state::SplitDirection::Right);
    assert_eq!(state.active_pane_id(), second);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert_eq!(state.active_cursor_pos(), redox_core::Pos::new(5, 0));
    state.apply_input(InputAction::Redo, 100, 29);
    assert_eq!(state.session.active_buffer().line_string(5), "xline");
}

#[test]
fn substitution_preview_preserves_scrolled_split_views_and_syntax() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    let source = format!(
        "// foo\n{}fn main() {{\n    let foo = 1;\n    let foo = 2;\n}}\n{}",
        "// heading\n".repeat(39),
        "// trailing\n".repeat(40)
    );
    *state.session.active_buffer_mut() = TextBuffer::from_text(&source);
    state.session.active_meta_mut().path = Some(PathBuf::from("preview.rs"));
    state.set_editor_area_size(100, 29);
    state.split_active_pane(crate::app::state::SplitAxis::Vertical);
    let pane_ids: Vec<_> = state.panes().iter().map(|pane| pane.id).collect();
    state.with_active_buffer_view_mut(|_, view| {
        view.cursor.cursor = redox_core::Pos::new(46, 0);
        view.cursor.scroll_y_lines = 41;
    });
    for pane_id in pane_ids {
        state.sync_rendered_pane_view(pane_id, state.session.active_id());
    }
    let mut clipboard = None;
    for character in r":s/foo/long_name\r    next/g".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    let style = UiStyle::default();
    let mut window = TestWindow::new(100, 30);
    let mut perf = FramePerfSample::default();
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    for rect in state.pane_rects(100, 29) {
        let preview_row = rect.y + 2;
        let row = window.row_text(preview_row);
        let pane_text: String = row
            .chars()
            .skip(rect.x as usize)
            .take(rect.width as usize)
            .collect();
        assert!(pane_text.contains("let long_name"), "{pane_text}");
        let column = rect.x as usize
            + pane_text[..pane_text.find("long_name").unwrap()]
                .chars()
                .count();
        let pane_style = if rect.pane_id == state.active_pane_id() {
            style
        } else {
            style.dimmed()
        };
        assert_eq!(
            window.backgrounds[preview_row as usize][column],
            Some(pane_style.substitute_colors(true).bg)
        );
        assert_eq!(
            window.foregrounds[preview_row as usize][column],
            Some(pane_style.substitute_colors(true).fg)
        );
        let keyword =
            rect.x as usize + pane_text[..pane_text.find("let ").unwrap()].chars().count();
        assert_eq!(
            window.foregrounds[preview_row as usize][keyword],
            Some(pane_style.syntax.keyword.fg)
        );
    }
    assert_eq!(state.session.active_buffer().to_string(), source);
    assert_eq!(
        state.views[&state.session.active_id()]
            .cursor
            .scroll_y_lines,
        41
    );
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    let mut window = TestWindow::new(100, 30);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert!(window.row_text(0).contains("let foo"));
    assert_eq!(
        state.views[&state.session.active_id()]
            .cursor
            .scroll_y_lines,
        41
    );
}

#[test]
fn calculator_ghost_text_inserts_on_enter_and_cancels_on_escape() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    let mut clipboard = None;
    for character in ":5+(2x3)".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    state.apply_input(InputAction::CommandMoveLeft, 80, 24);
    state.apply_input(InputAction::CommandBackspace, 80, 24);
    state.apply_input(InputAction::CommandChar('4'), 80, 24);
    let style = UiStyle::default();
    let mut window = TestWindow::new(80, 24);
    let mut perf = FramePerfSample::default();
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    let row = (0..24)
        .find(|row| window.row_text(*row).contains("5+(2x4) = 13"))
        .expect("calculation preview");
    let line = window.row_text(row);
    let answer = line[..line.find("13").unwrap()].chars().count();
    assert_eq!(
        window.foregrounds[usize::from(row)][answer],
        Some(style.command_line.ghost.fg)
    );
    let cursor = window.cursor.as_ref().expect("command cursor");
    assert_eq!(
        window.cells[usize::from(cursor.y)][usize::from(cursor.x)],
        ')'
    );
    assert!(state.session.active_buffer().to_string().is_empty());
    handle_editor_event(&mut state, &mut clipboard, Event::Escape);
    assert_eq!(state.mode, crate::app::EditorMode::Normal);
    assert!(state.session.active_buffer().to_string().is_empty());
    for character in ":5+(2x4)".chars() {
        handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    assert_eq!(state.mode, crate::app::EditorMode::Normal);
    assert_eq!(state.session.active_buffer().to_string(), "13");
    assert_eq!(state.status_msg, None);
}

#[test]
fn command_completion_renders_after_input_and_accepts_from_inside_it() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    state.configure_command_completions(["étoile", "été"]);
    for (prefix, suffix) in [
        ("config re", "load"),
        ("  colorscheme é", "toile"),
        ("conver", "t"),
    ] {
        state.apply_input(InputAction::EnterCommand, 80, 24);
        for character in prefix.chars() {
            state.apply_input(InputAction::CommandChar(character), 80, 24);
        }
        state.apply_input(InputAction::CommandMoveLeft, 80, 24);
        state.apply_input(InputAction::CommandCompletionNext, 80, 24);
        state.apply_input(InputAction::CommandCompletionPrev, 80, 24);
        assert_eq!(state.command_completion_suffix(), Some(suffix));

        let style = UiStyle::default();
        let mut window = TestWindow::new(80, 24);
        let mut perf = FramePerfSample::default();
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        let expected = format!("{prefix}{suffix}");
        let row = (0..24)
            .find(|row| window.row_text(*row).contains(&expected))
            .expect("completion preview after the full input");
        let line = window.row_text(row);
        let input_start = line[..line.find(&expected).unwrap()].chars().count();
        assert_eq!(
            window.foregrounds[usize::from(row)][input_start + prefix.chars().count()],
            Some(style.command_line.ghost.fg)
        );
        assert_eq!(
            usize::from(window.cursor.as_ref().unwrap().x),
            input_start + prefix[..state.command_line_cursor].chars().count()
        );

        state.apply_input(InputAction::CommandComplete, 80, 24);
        assert_eq!(state.command_line, expected);
        assert_eq!(state.command_line_cursor, expected.len());
        assert_eq!(state.mode, crate::app::EditorMode::Command);
    }
}

fn temp_dir_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_nanos();
    std::env::temp_dir().join(format!("redox_lib_test_{tag}_{nanos}"))
}

#[test]
fn layout_changes_preserve_cursor_and_buffer_position() {
    use crate::app::state::{SplitAxis, SplitDirection};
    use redox_core::Pos;
    let _guard = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() =
        TextBuffer::from_text(&format!("{}\n", "x".repeat(160)).repeat(100));
    let directory = tempfile::tempdir().unwrap();
    state.terminal = crate::terminal::tests::configured_panel(directory.path());
    state.with_active_buffer_view_mut(|_, view| {
        view.cursor.cursor = Pos::new(35, 90);
        view.cursor.scroll_x_cells = 10;
        view.cursor.scroll_y_lines = 10;
    });
    let mut clipboard = None;
    let mut window = TestWindow::new(100, 36);
    let mut perf = FramePerfSample::default();
    let check = |state: &mut EditorState, window: &mut TestWindow, perf: &mut FramePerfSample| {
        draw_buffer_view(state, &UiStyle::default(), window, perf).unwrap();
        state.with_active_buffer_view_mut(|_, view| {
            assert_eq!(view.cursor.cursor, Pos::new(35, 90));
            assert_eq!(view.cursor.viewport_scroll(), (10, 10));
        });
    };
    check(&mut state, &mut window, &mut perf);
    let control = |key| {
        Event::KeyWithModifiers(minui::KeyWithModifiers {
            key,
            mods: minui::KeyModifiers::ctrl(),
        })
    };
    for key in [
        KeyKind::Char('`'),
        KeyKind::Up,
        KeyKind::Down,
        KeyKind::Char('k'),
        KeyKind::Char('`'),
    ] {
        handle_editor_event(&mut state, &mut clipboard, control(key));
        check(&mut state, &mut window, &mut perf);
    }
    for (axis, direction) in [
        (SplitAxis::Vertical, SplitDirection::Left),
        (SplitAxis::Horizontal, SplitDirection::Up),
    ] {
        state.split_active_pane(axis);
        state.focus_split(direction);
        check(&mut state, &mut window, &mut perf);
        state.resize_active_pane(direction);
        check(&mut state, &mut window, &mut perf);
    }
    let mut narrow = TestWindow::new(60, 20);
    check(&mut state, &mut narrow, &mut perf);
    handle_editor_event(&mut state, &mut clipboard, Event::Character('h'));
    draw_buffer_view(&mut state, &UiStyle::default(), &mut narrow, &mut perf).unwrap();
    assert_eq!(state.active_cursor_pos(), Pos::new(35, 89));
    assert!(narrow.cursor.unwrap().visible);
}

#[test]
fn terminal_panel_routes_input_and_reserves_bottom_rows() {
    let _guard = crate::app::state::global_test_state_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    let directory = tempfile::tempdir().unwrap();
    state.terminal = crate::terminal::tests::configured_panel(directory.path());
    let mut clipboard = None;
    let mut window = TestWindow::new(100, 30);
    let mut perf = FramePerfSample::default();
    draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf).unwrap();
    let toggle = Event::KeyWithModifiers(minui::KeyWithModifiers {
        key: KeyKind::Char('`'),
        mods: minui::KeyModifiers::ctrl(),
    });
    handle_editor_event(&mut state, &mut clipboard, toggle.clone());
    assert!(state.terminal.is_focused());
    assert!(state.next_wake_deadline(Instant::now()).is_some());
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::Paste("printf '\\033[31m__%s__\\033[38;2;80;160;240mRGB\\033[1mB\\033[3mI\\033[4mU\\033[22mN\\033[23;24mP\\033[2mD\\033[22;7mR\\033[0mZ' ROUTED".into()),
    );
    handle_editor_event(&mut state, &mut clipboard, Event::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        state.update_background(Instant::now());
        window.clear_screen().unwrap();
        draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf).unwrap();
        if (20..30).any(|row| window.row_text(row).contains("__ROUTED__")) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    state.update_background(Instant::now() + Duration::from_millis(150));
    draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf).unwrap();
    assert!(window.row_text(19).contains("TERMINAL"));
    assert!(!window.row_text(20).contains("Ctrl+` hide"));
    assert!(window.cursor.unwrap().y >= 20);
    let output_row = (20..30)
        .find(|row| window.row_text(*row).contains("__ROUTED__"))
        .unwrap();
    let output_column = window.row_text(output_row).find("__ROUTED__").unwrap();
    let colors = [Color::AnsiValue(1), Color::rgb(80, 160, 240)];
    for (offset, color) in [0, 10].into_iter().zip(colors) {
        assert_eq!(
            window.foregrounds[output_row as usize][output_column + offset],
            Some(color)
        );
    }
    use crossterm::style::Attribute;
    let expected_attributes = [
        (13, vec![Attribute::Bold]),
        (14, vec![Attribute::Bold, Attribute::Italic]),
        (
            15,
            vec![Attribute::Bold, Attribute::Italic, Attribute::Underlined],
        ),
        (16, vec![Attribute::Italic, Attribute::Underlined]),
        (17, vec![]),
        (18, vec![Attribute::Dim]),
        (19, vec![]),
        (20, vec![]),
    ];
    for (offset, expected) in &expected_attributes {
        assert_eq!(
            window.styles[output_row as usize][output_column + offset]
                .to_crossterm()
                .attributes,
            crossterm::style::Attributes::from(expected.as_slice()),
        );
    }
    assert_eq!(
        window.foregrounds[output_row as usize][output_column + 19],
        Some(UiStyle::default().theme.bg)
    );
    assert_eq!(
        window.backgrounds[output_row as usize][output_column + 19],
        Some(colors[1])
    );
    let plain_config: crate::config::Config = toml::from_str("text_formatting = false").unwrap();
    draw_buffer_view(
        &mut state,
        &plain_config.style().unwrap(),
        &mut window,
        &mut perf,
    )
    .unwrap();
    for offset in 0..=20 {
        assert_eq!(
            window.styles[output_row as usize][output_column + offset]
                .to_crossterm()
                .attributes,
            crossterm::style::Attributes::default()
        );
    }
    assert_eq!(
        window.foregrounds[output_row as usize][output_column + 19],
        Some(colors[1])
    );
    assert_eq!(
        window.backgrounds[output_row as usize][output_column + 19],
        Some(UiStyle::default().theme.bg)
    );
    assert_eq!(state.mode, crate::app::EditorMode::Normal);
    assert_eq!(state.session.active_buffer().to_string(), "");
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 2,
            y: 2,
            button: minui::MouseButton::Left,
        },
    );
    assert!(!state.terminal.is_focused());
    state.sync_focus(Instant::now());
    state.update_background(Instant::now() + Duration::from_millis(150));
    draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf).unwrap();
    let style = UiStyle::default();
    for (offset, expected) in &expected_attributes {
        assert_eq!(
            window.styles[output_row as usize][output_column + offset]
                .to_crossterm()
                .attributes,
            crossterm::style::Attributes::from(expected.as_slice()),
        );
    }
    for (offset, color) in [0, 10]
        .into_iter()
        .zip([Color::rgb(205, 49, 49), colors[1]])
    {
        assert_eq!(
            window.foregrounds[output_row as usize][output_column + offset],
            Some(crate::ui::style::dim_foreground_color(
                color,
                style.theme.bg,
                style.dim_amount
            ))
        );
    }
    handle_editor_event(&mut state, &mut clipboard, Event::Character('i'));
    handle_editor_event(&mut state, &mut clipboard, Event::Character('x'));
    assert_eq!(state.session.active_buffer().to_string(), "x");
    handle_editor_event(&mut state, &mut clipboard, Event::Paste("\nx".into()));
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 2,
            y: 22,
            button: minui::MouseButton::Left,
        },
    );
    assert!(state.terminal.is_focused());
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::KeyWithModifiers(minui::KeyWithModifiers {
            key: KeyKind::Char('c'),
            mods: minui::KeyModifiers::ctrl(),
        }),
    );
    assert_eq!(state.mode, crate::app::EditorMode::Insert);
    let navigation = |character| {
        Event::KeyWithModifiers(minui::KeyWithModifiers {
            key: KeyKind::Char(character),
            mods: minui::KeyModifiers::ctrl(),
        })
    };
    handle_editor_event(&mut state, &mut clipboard, navigation('k'));
    assert!(!state.terminal.is_focused());
    assert_eq!(state.mode, crate::app::EditorMode::Insert);
    state.split_active_pane(crate::app::state::SplitAxis::Horizontal);
    draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf).unwrap();
    let lower_pane = state.active_pane_id();
    handle_editor_event(&mut state, &mut clipboard, navigation('k'));
    let upper_pane = state.active_pane_id();
    assert_ne!(upper_pane, lower_pane);
    handle_editor_event(&mut state, &mut clipboard, navigation('j'));
    assert_eq!(state.active_pane_id(), lower_pane);
    assert!(!state.terminal.is_focused());
    handle_editor_event(&mut state, &mut clipboard, navigation('j'));
    assert!(state.terminal.is_focused());
    handle_editor_event(&mut state, &mut clipboard, navigation('j'));
    assert!(state.terminal.is_focused());
    handle_editor_event(&mut state, &mut clipboard, navigation('k'));
    assert!(!state.terminal.is_focused());
    assert_eq!(state.active_pane_id(), lower_pane);
    handle_editor_event(&mut state, &mut clipboard, navigation('j'));
    assert!(state.terminal.is_focused());
    let style = UiStyle::default();
    state.sync_focus(Instant::now());
    state.update_background(Instant::now() + Duration::from_millis(150));
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    for rect in state.pane_rects(100, 19) {
        let (row, column) = (rect.y..rect.y + rect.height)
            .find_map(|row| {
                (rect.x..rect.x + rect.width)
                    .find(|column| window.cells[row as usize][*column as usize] == 'x')
                    .map(|column| (row, column))
            })
            .unwrap();
        assert_eq!(
            window.foregrounds[row as usize][column as usize],
            Some(style.dimmed().theme.white)
        );
    }
    for (key, height) in [(KeyKind::Up, 11), (KeyKind::Down, 10)] {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::KeyWithModifiers(minui::KeyWithModifiers {
                key,
                mods: minui::KeyModifiers::ctrl(),
            }),
        );
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert_eq!(state.terminal.height(30), height);
    }
    state.configure_mouse(true, false, false, 3, 3);
    state.zen.enabled = true;
    state.zen.width_percent = 80;
    state.zen.min_width = 1;
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    for row in 0..30 {
        for column in (0..10).chain(90..100) {
            assert_eq!(window.backgrounds[row][column], Some(style.zen_margin));
        }
    }
    assert!((10..90).contains(&window.cursor.unwrap().x));
    for event in [
        Event::MouseClick {
            x: 0,
            y: 19,
            button: minui::MouseButton::Left,
        },
        Event::MouseDrag {
            x: 0,
            y: 15,
            button: minui::MouseButton::Left,
        },
        Event::MouseRelease {
            x: 0,
            y: 15,
            button: minui::MouseButton::Left,
        },
    ] {
        handle_editor_event(&mut state, &mut clipboard, event);
    }
    assert_eq!(state.terminal.height(30), 10);
    assert!(state.terminal.is_focused());
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseClick {
            x: 10,
            y: 19,
            button: minui::MouseButton::Left,
        },
    );
    for (row, height) in [(15, 14), (0, 22), (13, 16)] {
        handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::MouseDrag {
                x: 0,
                y: row,
                button: minui::MouseButton::Left,
            },
        );
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert_eq!(state.terminal.height(30), height);
        assert!(state.terminal.is_focused());
    }
    handle_editor_event(
        &mut state,
        &mut clipboard,
        Event::MouseRelease {
            x: 0,
            y: 13,
            button: minui::MouseButton::Left,
        },
    );
    assert_eq!(state.mode, crate::app::EditorMode::Insert);
    handle_editor_event(&mut state, &mut clipboard, toggle.clone());
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    handle_editor_event(&mut state, &mut clipboard, toggle.clone());
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
    assert_eq!(state.terminal.height(30), 16);
    for (width, height) in [(100, 20), (2, 3), (1, 1), (0, 0)] {
        let mut small = TestWindow::new(width, height);
        draw_buffer_view(&mut state, &UiStyle::default(), &mut small, &mut perf).unwrap();
        if let Some(cursor) = small.cursor
            && cursor.visible
        {
            assert!(cursor.x < width && cursor.y < height);
        }
    }
    handle_editor_event(&mut state, &mut clipboard, toggle);
    assert!(!state.terminal.is_focused());
    assert_eq!(state.terminal.height(30), 0);
    assert_eq!(state.mode, crate::app::EditorMode::Insert);
}

pub(crate) struct TestWindow {
    width: u16,
    height: u16,
    cells: Vec<Vec<char>>,
    backgrounds: Vec<Vec<Option<Color>>>,
    foregrounds: Vec<Vec<Option<Color>>>,
    pub(crate) styles: Vec<Vec<minui::Style>>,
    cursor: Option<minui::window::CursorSpec>,
}

impl TestWindow {
    pub(crate) fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cells: vec![vec![' '; width as usize]; height as usize],
            backgrounds: vec![vec![None; width as usize]; height as usize],
            foregrounds: vec![vec![None; width as usize]; height as usize],
            styles: vec![vec![minui::Style::new(); width as usize]; height as usize],
            cursor: None,
        }
    }

    pub(crate) fn row_text(&self, row: u16) -> String {
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
        colors: minui::ColorPair,
    ) -> minui::Result<()> {
        self.write_text(y, x, s);
        if let Some(row) = self.backgrounds.get_mut(y as usize) {
            for cell in row.iter_mut().skip(x as usize).take(s.chars().count()) {
                *cell = Some(colors.bg);
            }
        }
        if let Some(row) = self.foregrounds.get_mut(y as usize) {
            for cell in row.iter_mut().skip(x as usize).take(s.chars().count()) {
                *cell = Some(colors.fg);
            }
        }
        if let Some(row) = self.styles.get_mut(y as usize) {
            for cell in row.iter_mut().skip(x as usize).take(s.chars().count()) {
                *cell = colors.into();
            }
        }
        Ok(())
    }

    fn write_str_styled(
        &mut self,
        y: u16,
        x: u16,
        text: &str,
        style: minui::Style,
    ) -> minui::Result<()> {
        self.write_str_colored(y, x, text, style.colors.unwrap_or_default())?;
        if let Some(row) = self.styles.get_mut(y as usize) {
            for cell in row.iter_mut().skip(x as usize).take(text.chars().count()) {
                *cell = style;
            }
        }
        Ok(())
    }

    fn request_cursor(&mut self, cursor: minui::window::CursorSpec) {
        self.cursor = Some(cursor);
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
fn configured_formatting_reaches_renderers_and_plain_mode() {
    use crate::ui::syntax::{SyntaxLanguage, line_spans_for_source};
    use crossterm::style::Attribute;

    let mut config: crate::config::Config = toml::from_str(
        r#"
[themes.default.syntax]
markdown_heading = { bold = false, underline = "curl" }
markdown_emphasis = { italic = false, strikethrough = true }
[themes.default.ui]
"editor.text" = { dim = true }
"dashboard.hotkey" = { bold = false, italic = true }
"about.logo_white" = { bold = false, strikethrough = true }
"diagnostic.error_range" = { underline = "none" }
"search.match" = { underline = "single", italic = true }
"finder.selected" = { italic = true }
"#,
    )
    .unwrap();
    for enabled in [true, false] {
        config.text_formatting = enabled;
        let style = config.style().unwrap();
        let mut window = TestWindow::new(40, 24);
        let source = "# *nested*";
        let spans = line_spans_for_source(source, Some(SyntaxLanguage::Markdown)).unwrap();
        let viewport = LineViewport {
            row: 0,
            column: 0,
            scroll_x: 0,
            width: 40,
        };
        draw_line_with_syntax(
            &mut window,
            viewport,
            source,
            style.editor_text,
            None,
            &style,
            &spans[0],
        )
        .unwrap();
        let attributes = window.styles[0][3].to_crossterm().attributes;
        assert!(!attributes.has(Attribute::Bold));
        assert!(!attributes.has(Attribute::Italic));
        assert_eq!(attributes.has(Attribute::Dim), enabled);
        assert_eq!(attributes.has(Attribute::CrossedOut), enabled);
        assert_eq!(attributes.has(Attribute::Undercurled), enabled);

        draw_plain_line(
            &mut window,
            viewport,
            "é\tplain",
            style.editor_text,
            Some((0, Color::Blue)),
        )
        .unwrap();
        assert_eq!(
            window.styles[0][0]
                .to_crossterm()
                .attributes
                .has(Attribute::Dim),
            enabled
        );
        assert_eq!(window.styles[0][0].colors.unwrap().bg, Color::Blue);

        let decorations = LineDecorations {
            search_cells: &[true],
            error_cells: &[true],
            current_cells: &[],
            search_style: style.search_match,
            current_style: style.search_current,
            error_style: style.error_range,
            yank_highlight: None,
            jump_highlight: None,
            delimiter_blink: None,
            undo_redo_highlight: None,
        };
        let decorated = decorations
            .apply(style.editor_text.into(), 0..1)
            .to_crossterm()
            .attributes;
        assert_eq!(decorated.has(Attribute::Underlined), enabled);
        assert!(!decorated.has(Attribute::Undercurled));
        assert_eq!(decorated.has(Attribute::Italic), enabled);

        window.clear_screen().unwrap();
        draw_dashboard(&mut window, &style, 0, false, 1.0).unwrap();
        let row = (0..24)
            .find(|row| window.row_text(*row).contains("Finder"))
            .unwrap();
        let column = window.row_text(row).rfind('f').unwrap();
        let attributes = window.styles[row as usize][column]
            .to_crossterm()
            .attributes;
        assert!(!attributes.has(Attribute::Bold));
        assert_eq!(attributes.has(Attribute::Italic), enabled);
        let row = (0..24)
            .find(|row| window.row_text(*row).contains("Redox"))
            .unwrap();
        let column = window.row_text(row).find("Redox").unwrap();
        let attributes = window.styles[row as usize][column]
            .to_crossterm()
            .attributes;
        assert!(!attributes.has(Attribute::Bold));
        assert_eq!(attributes.has(Attribute::CrossedOut), enabled);
        if !enabled {
            assert!(
                window
                    .styles
                    .iter()
                    .flatten()
                    .all(|style| style.to_crossterm().attributes
                        == crossterm::style::Attributes::default())
            );
        }
    }
}

#[test]
fn syntax_decorations_survive_search_and_selection_without_leaking() {
    use crate::ui::syntax::{SyntaxLanguage, line_spans_for_source};
    use crossterm::style::Attribute;

    let mut style = UiStyle::default();
    let colors = TextStyle::new(style.theme.white, style.theme.bg);
    // Equal colours must still produce separate runs when attributes change.
    style.syntax.comment = style.syntax.comment.with_colors(colors.fg, colors.bg);
    style.syntax.markdown_emphasis = style
        .syntax
        .markdown_emphasis
        .with_colors(colors.fg, colors.bg);
    style.syntax.markdown_strong = style
        .syntax
        .markdown_strong
        .with_colors(colors.fg, colors.bg);
    style.syntax.markdown_heading = style
        .syntax
        .markdown_heading
        .with_colors(colors.fg, colors.bg);
    for (source, language, checks) in [
        (
            "plain *soft* **strong** ***both*** plain",
            Some(SyntaxLanguage::Markdown),
            vec![
                ("plain", false, false),
                ("soft", false, true),
                ("strong", true, false),
                ("both", true, true),
            ],
        ),
        (
            "# Heading *soft*",
            Some(SyntaxLanguage::Markdown),
            vec![("Heading", true, false), ("soft", true, true)],
        ),
        (
            "let value = 1; // comment",
            Some(SyntaxLanguage::Rust),
            vec![("let", false, false), ("comment", false, true)],
        ),
        (
            "value // comment",
            None,
            vec![("value", false, false), ("comment", false, true)],
        ),
    ] {
        let spans = line_spans_for_source(source, language)
            .map(|lines| lines.into_iter().next().unwrap())
            .unwrap_or_else(|| lexical_fallback_line_spans(source));
        let selected = selected_visible_cells(source, 0, 80, 0, source.len());
        for searched in [false, true] {
            let mut window = TestWindow::new(80, 1);
            let viewport = LineViewport {
                row: 0,
                column: 0,
                scroll_x: 0,
                width: 80,
            };
            if searched {
                draw_line_with_highlights(
                    &mut window,
                    viewport,
                    source,
                    colors,
                    LineHighlights {
                        color_column: None,
                        layers: &[(&selected, style.theme.selection_bg, None)],
                        empty_line: false,
                        decorations: LineDecorations {
                            search_cells: &selected,
                            error_cells: &[],
                            current_cells: &[],
                            search_style: style.search_match,
                            current_style: style.search_current,
                            error_style: style.error_range,
                            yank_highlight: None,
                            jump_highlight: None,
                            delimiter_blink: None,
                            undo_redo_highlight: None,
                        },
                    },
                    &style,
                    Some(&spans),
                )
                .unwrap();
            } else {
                draw_line_with_syntax(&mut window, viewport, source, colors, None, &style, &spans)
                    .unwrap();
            }
            for (word, bold, italic) in &checks {
                for (column, _) in source.match_indices(word) {
                    let attributes = window.styles[0][column].to_crossterm().attributes;
                    assert_eq!(attributes.has(Attribute::Bold), *bold, "{source}: {word}");
                    assert_eq!(
                        attributes.has(Attribute::Italic),
                        *italic,
                        "{source}: {word}"
                    );
                    assert_eq!(
                        attributes.has(Attribute::Underlined),
                        searched || source.starts_with('#')
                    );
                }
            }
            assert_eq!(window.styles[0][source.len()], minui::Style::new());
        }
    }
}

#[test]
fn diagnostic_undercurl_survives_selection_and_source_overlays() {
    use crossterm::style::Attribute;

    let style = UiStyle::default();
    let colors = TextStyle::new(style.theme.white, style.theme.bg);
    let source = "x(a)bc";
    for scroll_x in [0, 2] {
        let viewport = LineViewport {
            row: 0,
            column: 0,
            scroll_x,
            width: 8,
        };
        let selected = selected_visible_cells(source, scroll_x, 8, 0, 6);
        let diagnostic = selected_visible_cells(source, scroll_x, 8, 1, 5);
        for error_cells in [diagnostic.as_slice(), &[]] {
            let decorations = LineDecorations {
                search_cells: &selected,
                error_cells,
                current_cells: &[],
                search_style: style.search_match,
                current_style: style.search_current,
                error_style: style.error_range,
                yank_highlight: None,
                jump_highlight: None,
                undo_redo_highlight: None,
                delimiter_blink: Some((
                    3 - scroll_x,
                    HighlightFade::new(0.0, style.editor_text.fg),
                )),
            };
            let mut window = TestWindow::new(8, 1);
            draw_line_with_highlights(
                &mut window,
                viewport,
                source,
                colors,
                LineHighlights {
                    color_column: None,
                    layers: &[(&selected, style.theme.selection_bg, None)],
                    empty_line: false,
                    decorations,
                },
                &style,
                None,
            )
            .unwrap();
            draw_delimiter_highlights(
                &mut window,
                viewport,
                source,
                (&[1, 3], decorations),
                colors,
                &style,
                None,
            )
            .unwrap();
            draw_snippet_placeholders(
                &mut window,
                viewport,
                source,
                std::slice::from_ref(&(4..5)),
                &style,
                decorations,
                None,
            )
            .unwrap();
            for column in 0..source.len() - scroll_x {
                let text_style = window.styles[0][column];
                let undercurled = !error_cells.is_empty() && (1..5).contains(&(column + scroll_x));
                assert_eq!(
                    text_style
                        .to_crossterm()
                        .attributes
                        .has(Attribute::Undercurled),
                    undercurled
                );
                assert_eq!(
                    text_style.underline_color,
                    undercurled.then_some(style.diagnostic_inline.error.fg)
                );
                assert_eq!(
                    text_style
                        .to_crossterm()
                        .attributes
                        .has(Attribute::Underlined),
                    !undercurled
                );
            }
            assert_eq!(
                window.backgrounds[0][2 - scroll_x],
                Some(style.theme.selection_bg)
            );
            assert_ne!(window.backgrounds[0][3 - scroll_x], Some(style.theme.scope));
        }
    }
    for source in ["", "text"] {
        let search_cells = search_highlight_cells(
            source,
            0,
            8,
            std::slice::from_ref(&(source.len()..source.len())),
        );
        let mut window = TestWindow::new(8, 1);
        draw_line_with_highlights(
            &mut window,
            LineViewport {
                row: 0,
                column: 0,
                scroll_x: 0,
                width: 8,
            },
            source,
            colors,
            LineHighlights {
                color_column: None,
                layers: &[(&search_cells, style.theme.selection_bg, None)],
                empty_line: true,
                decorations: LineDecorations {
                    search_cells: &search_cells,
                    error_cells: &[],
                    current_cells: &[],
                    search_style: style.search_match,
                    current_style: style.search_current,
                    error_style: style.error_range,
                    yank_highlight: None,
                    jump_highlight: None,
                    delimiter_blink: None,
                    undo_redo_highlight: None,
                },
            },
            &style,
            None,
        )
        .unwrap();
        for column in 0..8 {
            assert_eq!(
                window.styles[0][column]
                    .to_crossterm()
                    .attributes
                    .has(Attribute::Underlined),
                column == source.len()
            );
        }
    }
}

#[test]
fn configured_line_numbers_render_scrolled_gutters() {
    for (setting, expected) in [
        ("relative", [" 2▕", " 1▕", "10▕", " 1▕", "   "]),
        ("absolute", [" 8▕", " 9▕", "10▕", "11▕", "   "]),
    ] {
        let config: crate::config::Config = toml::from_str(&format!(
            "line_numbers = {setting:?}\n[themes.custom.palette]\nbackground = \"#010203\""
        ))
        .unwrap();
        for theme in ["default", "custom"] {
            let style = config.style_for_theme(theme).unwrap();
            // Editor Git markers use one column; explorer status dots use two.
            for marker_width in 0..=2 {
                let gutter_width = line_number_gutter_width(11, marker_width);
                let mut window = TestWindow::new(gutter_width, 5);
                draw_line_numbers(
                    &mut window,
                    &style,
                    (gutter_width, 5),
                    marker_width,
                    7..11,
                    9,
                    None,
                )
                .unwrap();
                for (row, expected) in expected.iter().enumerate() {
                    assert_eq!(
                        window.row_text(row as u16),
                        format!("{}{expected}", " ".repeat(marker_width as usize)),
                        "{setting}, {theme}, marker width {marker_width}, row {row}"
                    );
                }
                assert_eq!(
                    window.foregrounds[2][marker_width as usize],
                    Some(style.theme.white)
                );
                assert_eq!(
                    window.foregrounds[1][marker_width as usize],
                    Some(style.theme.dark_gray)
                );
            }
        }
    }
    assert!(toml::from_str::<crate::config::Config>("line_numbers = \"invalid\"").is_err());
}

#[test]
fn draw_snapshot_lines_renders_scrolled_plain_text_without_double_scroll() {
    let buffer = redox_core::TextBuffer::from_text("abcdefghijklmnopqrstuvwxyz\n");
    let mut render_line_cache = crate::ui::RenderLineCache::new(1);
    let snapshot = render_line_cache.snapshot(
        &buffer,
        &TextViewport {
            scroll_x: 8,
            scroll_y: 0,
            width: 8,
            height: 1,
        },
    );
    let mut window = TestWindow::new(8, 1);
    let style = UiStyle::default();
    let default_colors = TextStyle::new(style.theme.white, style.theme.bg);

    draw_snapshot_lines(
        &mut window,
        &buffer,
        &snapshot,
        SnapshotViewport {
            content_x: 0,
            scroll_x: 8,
            text_w: 8,
        },
        SnapshotStyle {
            default_colors,
            style: &style,
            syntax_spans: None,
            lexical_fallback_enabled: false,
        },
        SnapshotOverlays {
            delimiter_highlights: &BTreeMap::new(),
            active_scope_guides: &BTreeMap::new(),
            search_highlights: &BTreeMap::new(),
            substitution_colors: None,
            snippet_placeholders: &BTreeMap::new(),
            diagnostic_lines: &BTreeMap::new(),
            visual_selection: None,
            one_shot_highlight: None,
            focused_lines: None,
        },
    )
    .expect("plain snapshot draw should succeed");

    assert_eq!(window.row_text(0), "ijklmnop");
}

#[test]
fn visual_selection_visible_cells_limit_highlights_to_contents() {
    let buffer = redox_core::TextBuffer::from_text("alpha\n\nomega\n");
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

    for (text, scroll_x, selected_width) in [
        ("alpha", 0, 5),
        ("", 0, 1),
        ("  a ", 0, 4),
        ("\t😁x", 0, 7),
        ("\t😁x", 5, 2),
        ("alpha", 5, 0),
        ("longer than the viewport", 0, 8),
    ] {
        let buffer = redox_core::TextBuffer::from_text(&format!("alpha\n{text}\nomega\n"));
        for mode in [
            redox_core::VisualModeKind::Line,
            redox_core::VisualModeKind::Char,
        ] {
            let cells =
                visual_selection_visible_cells(&buffer, text, selection, mode, 1, scroll_x, 8)
                    .unwrap_or_else(|| vec![false; 8]);
            assert_eq!(
                cells,
                (0..8).map(|cell| cell < selected_width).collect::<Vec<_>>(),
                "{mode:?}, {text:?}, scroll {scroll_x}"
            );
        }
    }

    for selection in [
        selection,
        redox_core::Selection::new(redox_core::Pos::new(2, 2), redox_core::Pos::new(0, 1)),
    ] {
        for (line, text, selected) in [(0, "alpha", 1..5), (2, "omega", 0..3)] {
            let cells = visual_selection_visible_cells(
                &buffer,
                text,
                selection,
                redox_core::VisualModeKind::Char,
                line,
                0,
                8,
            )
            .unwrap();
            assert_eq!(
                cells,
                (0..8)
                    .map(|cell| selected.contains(&cell))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn pane_zoom_preserves_layout_and_navigates_hidden_neighbours() {
    use crate::app::state::SplitAxis;
    use redox_core::Pos;
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    state.set_editor_area_size(100, 29);
    let left_top = state.active_pane_id();
    *state.session.active_buffer_mut() = TextBuffer::from_text(&"left-top\n".repeat(40));
    state.session.active_meta_mut().display_name = "left-top.txt".into();
    for (axis, name) in [
        (SplitAxis::Vertical, "right-top"),
        (SplitAxis::Horizontal, "right-bottom"),
    ] {
        state.split_active_pane(axis);
        state.session.open_unnamed_buffer();
        *state.session.active_buffer_mut() = TextBuffer::from_text(&format!("{name}\n").repeat(40));
        state.session.active_meta_mut().display_name = format!("{name}.txt");
        state.session.set_active_dirty(true);
        state.sync_active_pane_view();
    }
    let right_bottom = state.active_pane_id();
    let right_top = state.panes()[1].id;
    state.with_active_buffer_view_mut(|_, view| view.cursor.cursor = Pos::new(4, 2));
    state.sync_active_pane_view();
    let original_layout: Vec<_> = state
        .pane_rects(100, 29)
        .into_iter()
        .map(|rect| (rect.pane_id, rect.x, rect.y, rect.width, rect.height))
        .collect();
    let mut clipboard = None;
    let control = |key| {
        Event::KeyWithModifiers(minui::KeyWithModifiers {
            key,
            mods: minui::KeyModifiers::ctrl(),
        })
    };

    state.apply_input(InputAction::RunCommand("zoom".into()), 100, 30);
    assert!(state.pane_is_zoomed());
    assert_eq!(state.viewport_size(), (100, 30));
    let mut perf = FramePerfSample::default();
    for (key, expected_pane, name) in [
        ('k', right_top, "right-top"),
        ('h', left_top, "left-top"),
        ('l', right_top, "right-top"),
        ('j', right_bottom, "right-bottom"),
    ] {
        handle_editor_event(&mut state, &mut clipboard, control(KeyKind::Char(key)));
        assert_eq!(
            state.active_pane_id(),
            expected_pane,
            "key={key}; layout={original_layout:?}"
        );
        assert!(state.pane_is_zoomed());
        assert_eq!(state.panes().len(), 3);
        let rects = state.pane_rects(100, 29);
        assert_eq!(rects.len(), 1);
        assert_eq!(
            (rects[0].x, rects[0].y, rects[0].width, rects[0].height),
            (0, 0, 100, 29)
        );
        for icons_enabled in [false, true] {
            let style = UiStyle {
                icons_enabled,
                ..UiStyle::default()
            };
            let mut window = TestWindow::new(100, 30);
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            assert!(window.row_text(1).contains(name));
            let marker = if icons_enabled {
                crate::ui::icons::ZOOM
            } else {
                "[zoom]"
            };
            assert!(
                window
                    .row_text(29)
                    .contains(&format!("{name}.txt {marker}"))
            );
            assert!(window.cursor.unwrap().visible);
        }
    }
    assert_eq!(state.active_cursor_pos(), Pos::new(4, 2));
    handle_editor_event(&mut state, &mut clipboard, control(KeyKind::Right));
    handle_editor_event(&mut state, &mut clipboard, control(KeyKind::Char('z')));
    assert!(!state.pane_is_zoomed());
    let restored: Vec<_> = state
        .pane_rects(100, 29)
        .into_iter()
        .map(|rect| (rect.pane_id, rect.x, rect.y, rect.width, rect.height))
        .collect();
    assert_eq!(restored, original_layout);
    assert!(state.session.any_dirty());
    assert_eq!(state.active_cursor_pos(), Pos::new(4, 2));

    handle_editor_event(&mut state, &mut clipboard, control(KeyKind::Char('z')));
    assert!(state.pane_is_zoomed());
    state.apply_input(InputAction::RunCommand("only".into()), 100, 30);
    assert!(!state.pane_is_zoomed());
    assert_eq!(state.panes().len(), 1);
    handle_editor_event(&mut state, &mut clipboard, control(KeyKind::Char('z')));
    assert!(!state.pane_is_zoomed());
    assert_eq!(state.session.summaries().len(), 3);
}

#[test]
fn inactive_pane_filenames_overlay_the_top_row_without_shifting_text() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    for (axis, focus_back) in [
        (
            crate::app::state::SplitAxis::Vertical,
            InputAction::SplitFocusLeft,
        ),
        (
            crate::app::state::SplitAxis::Horizontal,
            InputAction::SplitFocusUp,
        ),
    ] {
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        let source = format!("first\n{}", "body\n".repeat(39));
        *state.session.active_buffer_mut() = TextBuffer::from_text(&source);
        state.session.active_meta_mut().path = Some(PathBuf::from("source/alpha.txt"));
        let style = UiStyle::default();
        let mut perf = FramePerfSample::default();
        let mut window = TestWindow::new(80, 12);
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        state.split_active_pane(axis);
        state.session.open_unnamed_buffer();
        *state.session.active_buffer_mut() = TextBuffer::from_text(&source);

        for (action, expected_filename) in
            [(InputAction::None, "alpha.txt"), (focus_back, "[No Name]")]
        {
            state.apply_input(action, 80, 12);
            state.update_background(Instant::now() + Duration::from_millis(150));
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            for rect in state.pane_rects(80, 11) {
                let row = rect.y as usize;
                let columns = rect.x as usize..(rect.x + rect.width) as usize;
                let text = window.cells[row][columns.clone()]
                    .iter()
                    .collect::<String>();
                let second_line = window.cells[row + 1][columns.clone()]
                    .iter()
                    .collect::<String>();
                assert!(second_line.contains("body"), "{second_line:?}");
                if rect.pane_id == state.active_pane_id() {
                    assert!(text.contains("first"), "{text:?}");
                } else {
                    assert_eq!(text.trim(), expected_filename);
                    assert_eq!(
                        text.find(expected_filename),
                        Some((rect.width as usize - expected_filename.len()) / 2)
                    );
                    assert!(
                        window.backgrounds[row][columns.clone()]
                            .iter()
                            .all(|color| { *color == Some(style.dimmed().pane_title.bg) })
                    );
                    assert!(
                        window.foregrounds[row][columns]
                            .iter()
                            .all(|color| { *color == Some(style.dimmed().pane_title.fg) })
                    );
                }
            }
            let cursor = window.cursor.unwrap();
            assert!(cursor.visible);
            assert_eq!(window.cells[cursor.y as usize][cursor.x as usize], 'f');
        }

        let active_buffer = state.session.active_id();
        let inactive_pane = state
            .panes()
            .iter()
            .find(|pane| pane.id != state.active_pane_id())
            .unwrap()
            .clone();
        for (path, dirty, expected_title) in [
            ("other/alpha.txt", true, "other/alpha.txt +"),
            ("other/beta.txt", false, "beta.txt"),
            ("source/alpha.txt", true, "alpha.txt +"),
            ("other/Alpha.txt", false, "Alpha.txt"),
        ] {
            state.session.activate(inactive_pane.buffer_id);
            state.session.active_meta_mut().path = Some(PathBuf::from(path));
            state.session.set_active_dirty(dirty);
            state.session.activate(active_buffer);
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            let rect = state
                .pane_rects(80, 11)
                .into_iter()
                .find(|rect| rect.pane_id == inactive_pane.id)
                .unwrap();
            let text = window.cells[rect.y as usize]
                [rect.x as usize..(rect.x + rect.width) as usize]
                .iter()
                .collect::<String>();
            assert_eq!(text.trim(), expected_title);
            assert_eq!(
                text.find(expected_title),
                Some((rect.width as usize - expected_title.len()) / 2)
            );
        }

        state.session.active_meta_mut().path = Some(PathBuf::from("crates/redox-core/src/lib.rs"));
        for (path, expected_title) in [
            ("crates/redox-tui/src/lib.rs", "redox-tui/src/lib.rs"),
            ("crates/redox-lsp/src/lib.rs", "redox-lsp/src/lib.rs"),
            ("elsewhere/redox-core/src/lib.rs", "redox-core/src/lib.rs"),
            ("crates/redox-core/src/lib.rs", "lib.rs"),
            ("crates/redox-lsp/tests/lib.rs", "tests/lib.rs"),
        ] {
            state.session.activate(inactive_pane.buffer_id);
            state.session.active_meta_mut().path = Some(PathBuf::from(path));
            state.session.activate(active_buffer);
            assert_eq!(
                inactive_pane_filename(&state, inactive_pane.buffer_id)
                    .unwrap()
                    .0,
                expected_title
            );
        }

        state.close_active_split();
        draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
        assert!(window.row_text(0).contains("first"));
        assert!(window.row_text(10).contains("body"));
        assert!(!window.row_text(0).contains("[No Name]"));
    }

    let mut narrow = TestWindow::new(8, 1);
    draw_pane_filename(
        &mut narrow,
        "é界🙂.txt",
        false,
        UiStyle::default().status_line.bar,
    )
    .unwrap();
    assert_eq!(narrow.row_text(0).trim(), "é界🙂.");
    assert_eq!(narrow.row_text(0).find('é'), Some(1));
    for width in [1, 2, 3, 8] {
        let mut narrow = TestWindow::new(width, 1);
        draw_pane_filename(
            &mut narrow,
            "é界🙂.txt",
            true,
            UiStyle::default().pane_title,
        )
        .unwrap();
        assert!(narrow.row_text(0).contains('+'));
    }
    let mut empty = TestWindow::new(0, 0);
    draw_pane_filename(
        &mut empty,
        "alpha.txt",
        true,
        UiStyle::default().status_line.bar,
    )
    .unwrap();
}

#[test]
fn zen_rendering_focuses_scope_and_restores_standard_layout() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let source =
        "fn first() {\n    let value = 1;\n}\n\nfn second() {\n    let other = 2; // comment\n}\n";
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(source);
    let meta = state.session.active_meta_mut();
    meta.path = Some(PathBuf::from("source/focus.rs"));
    meta.display_name = "source/focus.rs".to_string();
    state.with_active_buffer_view_mut(|buffer, view| {
        view.cursor.cursor = redox_core::Pos::new(1, 8);
        view.syntax_highlighter
            .replace_cache(crate::ui::syntax::SyntaxHighlighter::compute_cache(
                buffer,
                crate::ui::syntax::SyntaxLanguage::Rust,
            ));
    });
    let config: crate::config::Config = toml::from_str(
        r##"
[themes.default.syntax]
keyword = { bold = true, italic = true, dim = true, reverse = true, strikethrough = true, underline = "curl", underline_color = "#abcdef" }
function = { bg = "#445566", underline = "single" }
number = { bold = true }
comment = { italic = false }
"##,
    )
    .unwrap();
    let style = config.style().unwrap();
    let mut perf = FramePerfSample::default();
    let mut standard = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut standard, &mut perf).unwrap();
    let content_x = pane_content_x(&state, state.active_pane_id()) as usize;
    assert!(content_x > 0);
    let keyword_color = standard.foregrounds[0][content_x];

    state.apply_input(InputAction::ToggleZen, 120, 24);
    assert_eq!(state.status_msg.as_deref(), Some("zen"));
    state.clear_status();
    let mut zen = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut zen, &mut perf).unwrap();
    assert_eq!(state.viewport_size(), (96, 24));
    assert_eq!(zen.row_text(0).find("fn first"), Some(12));
    assert_eq!(zen.foregrounds[0][12], keyword_color);
    assert_eq!(zen.foregrounds[4][12], Some(style.zen_ghost));
    let ghost_colors = minui::ColorPair::new(style.zen_ghost, style.editor_text.bg);
    for (row, token, role) in [
        (4, "fn", crate::ui::style::SyntaxRole::Keyword),
        (4, "second", crate::ui::style::SyntaxRole::Function),
        (5, "//", crate::ui::style::SyntaxRole::Comment),
    ] {
        let column = zen.row_text(row).find(token).unwrap();
        assert_eq!(
            zen.styles[row as usize][column],
            minui::Style::from(style.syntax.color_for(role)).with_colors(ghost_colors),
            "{token} should keep its syntax formatting while dimmed"
        );
    }
    assert_eq!(zen.styles[4][14], minui::Style::from(ghost_colors));
    assert_eq!(zen.styles[0][12], standard.styles[0][content_x]);

    state.session.active_meta_mut().path = None;
    let mut fallback = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut fallback, &mut perf).unwrap();
    let column = fallback.row_text(5).find('2').unwrap();
    assert_eq!(
        fallback.styles[5][column],
        minui::Style::from(style.syntax.number).with_colors(ghost_colors)
    );
    state.session.active_meta_mut().path = Some(PathBuf::from("source/focus.rs"));
    assert_eq!(zen.backgrounds[0][0], Some(style.zen_margin));
    assert_eq!(zen.backgrounds[0][119], Some(style.zen_margin));
    assert_eq!(
        zen.backgrounds[3][12 + style.layout.color_column.unwrap()],
        Some(style.theme.bg)
    );
    let cursor = zen.cursor.unwrap();
    assert!(cursor.visible);
    assert_eq!((cursor.x, cursor.y), (20, 1));
    assert!(zen.row_text(23).contains("focus.rs"));
    assert!(!zen.row_text(23).contains('/'));
    assert!(zen.row_text(23).contains("2:9"));

    state.zen.focus_scope = false;
    state.zen.hide_gutter = false;
    state.zen.hide_color_column = false;
    let mut configured = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut configured, &mut perf).unwrap();
    assert_eq!(
        configured.cells[0]
            .iter()
            .position(|character| *character == 'f'),
        Some(12 + content_x)
    );
    assert_eq!(configured.foregrounds[4][12 + content_x], keyword_color);
    assert_eq!(
        configured.backgrounds[3][12 + content_x + style.layout.color_column.unwrap()],
        Some(style.theme.color_column)
    );

    state.apply_input(InputAction::ToggleZen, 120, 24);
    assert_eq!(state.status_msg.as_deref(), Some("standard"));
    state.clear_status();
    let mut restored = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut restored, &mut perf).unwrap();
    assert_eq!(restored.cells, standard.cells);
    assert_eq!(restored.foregrounds, standard.foregrounds);
    assert_eq!(restored.backgrounds, standard.backgrounds);

    state.zen.show_toast = false;
    state.toggle_zen();
    assert!(state.status_msg.is_none());
    state.zen.hide_gutter = true;
    for (width, expected) in [(90, 80), (50, 50), (1, 1), (0, 0)] {
        let mut narrow = TestWindow::new(width, 24);
        draw_buffer_view(&mut state, &style, &mut narrow, &mut perf).unwrap();
        assert_eq!(state.viewport_size().0, expected);
    }

    *state.session.active_buffer_mut() = TextBuffer::from_text(&"x".repeat(160));
    state.with_active_buffer_view_mut(|_, view| {
        view.cursor.cursor = redox_core::Pos::new(0, 115);
        view.cursor.scroll_x_cells = 0;
    });
    let mut narrow = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut narrow, &mut perf).unwrap();
    let cursor = narrow.cursor.unwrap();
    assert!(!cursor.visible);
    assert_eq!(state.active_cursor_pos(), redox_core::Pos::new(0, 115));
    state.apply_input(
        InputAction::Motion {
            motion: redox_core::motion::Motion::Left,
            count: 1,
        },
        96,
        24,
    );
    draw_buffer_view(&mut state, &style, &mut narrow, &mut perf).unwrap();
    let cursor = narrow.cursor.unwrap();
    assert!(cursor.visible && (12..108).contains(&cursor.x));

    state.apply_input(InputAction::SplitVertical, 96, 24);
    assert_eq!(state.panes().len(), 2);
    let mut split = TestWindow::new(120, 24);
    draw_buffer_view(&mut state, &style, &mut split, &mut perf).unwrap();
    let cursor = split.cursor.unwrap();
    assert!(cursor.visible && (12..108).contains(&cursor.x));
    assert_eq!(split.cells[cursor.y as usize][cursor.x as usize], 'x');
    assert_eq!(split.backgrounds[0][0], Some(style.zen_margin));
}

#[test]
fn scope_overlays_follow_headers_in_standard_and_zen_views() {
    let _lock = crate::app::state::global_test_state_lock().lock().unwrap();
    let source = "fn main() {\n    match value {\n        Some(value) => {\n            let result = if ready {\n                value\n            } else {\n                fallback\n            };\n        }\n        None => Command::new(\"tool\")\n            .args([\n                \"run\",\n                \"output\",\n            ])\n            .output(),\n    }\n}\n";
    let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
    *state.session.active_buffer_mut() = TextBuffer::from_text(source);
    state.session.active_meta_mut().path = Some(PathBuf::from("scope.rs"));
    state.with_active_buffer_view_mut(|buffer, view| {
        view.syntax_highlighter
            .replace_cache(crate::ui::syntax::SyntaxHighlighter::compute_cache(
                buffer,
                crate::ui::syntax::SyntaxLanguage::Rust,
            ));
        view.delimiter_pair_cache
            .install(crate::ui::overlays::compute_delimiter_analysis(buffer));
    });
    let style = UiStyle::default();
    let mut perf = FramePerfSample::default();
    for zen in [false, true] {
        state.zen.enabled = zen;
        for (cursor, lines, opening_line, closing_line, indent, delimiters) in [
            (redox_core::Pos::new(1, 4), 1..16, 1, 15, 4, ('{', '}')),
            (redox_core::Pos::new(2, 8), 2..9, 2, 8, 8, ('{', '}')),
            (redox_core::Pos::new(3, 12), 3..8, 3, 5, 12, ('{', '}')),
            (redox_core::Pos::new(5, 14), 5..8, 5, 7, 12, ('{', '}')),
            (redox_core::Pos::new(7, 12), 5..8, 5, 7, 12, ('{', '}')),
            (redox_core::Pos::new(7, 13), 3..8, 5, 7, 12, ('{', '}')),
            (redox_core::Pos::new(10, 14), 9..14, 10, 13, 12, ('(', ')')),
        ] {
            state.with_active_buffer_view_mut(|_, view| view.cursor.cursor = cursor);
            let mut window = TestWindow::new(120, 24);
            draw_buffer_view(&mut state, &style, &mut window, &mut perf).unwrap();
            let content_x = window.cells[0]
                .iter()
                .position(|character| *character == 'f')
                .unwrap();
            let opening_col = source
                .lines()
                .nth(opening_line)
                .unwrap()
                .rfind(delimiters.0)
                .unwrap();
            let closing_col = source
                .lines()
                .nth(closing_line)
                .unwrap()
                .rfind(delimiters.1)
                .unwrap();
            assert_eq!(
                window.cells[opening_line + 1][content_x + indent],
                '│',
                "{cursor:?}, zen={zen}: {:?}",
                window.row_text((opening_line + 1) as u16)
            );
            for (line, column) in [(opening_line, opening_col), (closing_line, closing_col)] {
                assert_eq!(
                    window.backgrounds[line][content_x + column],
                    Some(style.theme.scope)
                );
            }
            for (line, cells) in window.cells.iter().enumerate() {
                for (column, character) in cells.iter().enumerate() {
                    if *character == '│' {
                        assert!(lines.contains(&line), "guide outside scope at {line}");
                        assert!(
                            (opening_line + 1..closing_line).contains(&line),
                            "guide outside delimiter pair at {line}, cursor={cursor:?}"
                        );
                        assert_eq!(column, content_x + indent);
                    }
                }
            }
            if zen {
                assert_eq!(window.foregrounds[0][content_x], Some(style.zen_ghost));
                assert_ne!(
                    window.foregrounds[cursor.line][content_x + cursor.col],
                    Some(style.zen_ghost)
                );
            }
        }
    }
}

#[test]
fn draw_buffer_view_shows_status_toast_while_explorer_is_open() {
    let _lock = crate::app::state::global_test_state_lock()
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
    draw_buffer_view(&mut state, &UiStyle::default(), &mut window, &mut perf)
        .expect("draw should succeed");

    let screen = (0..24)
        .map(|row| window.row_text(row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("explorer write failed"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn draw_buffer_view_shrinks_explorer_to_stack_command_line() {
    let _lock = crate::app::state::global_test_state_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = temp_dir_path("explorer_command_line");
    fs::create_dir(&dir).expect("failed to create temp dir");
    fs::write(dir.join("alpha.txt"), "alpha").expect("failed to write fixture");

    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);
    state
        .open_explorer_at_path(dir.clone())
        .expect("failed to open explorer");
    state.apply_input(InputAction::EnterCommand, 80, 24);
    for character in "write".chars() {
        state.apply_input(InputAction::CommandChar(character), 80, 24);
    }

    let style = UiStyle::default();
    let (inner_w, inner_h) = explorer_popup_inner_size(80, 24, &style);
    let (_, popup_y) = anchored_popup_origin(80, 24, inner_w, inner_h);
    let command_outer_h = style
        .command_line
        .inner_height_rows
        .max(1)
        .saturating_add(2);
    let command_y = popup_y
        .saturating_add(inner_h)
        .saturating_add(2)
        .saturating_sub(command_outer_h);
    let mut window = TestWindow::new(80, 24);
    let mut perf = FramePerfSample::default();
    let stacked = popup_stack_layout(&state, &style, &window, (inner_w, inner_h));
    assert_eq!(stacked.popup.inner_h, inner_h - command_outer_h);
    draw_buffer_view(&mut state, &style, &mut window, &mut perf).expect("draw should succeed");

    assert!(window.row_text(command_y.saturating_sub(1)).contains('╰'));
    assert!(window.row_text(command_y).contains("Command"));
    assert!(
        window
            .row_text(command_y.saturating_add(1))
            .contains("write")
    );

    state.zen.enabled = true;
    let mut zen_window = TestWindow::new(120, 24);
    let (zen_width, zen_height) = explorer_popup_inner_size(96, 24, &style);
    let zen_stack = popup_stack_layout(
        &state,
        &style,
        &TestWindow::new(96, 24),
        (zen_width, zen_height),
    );
    draw_buffer_view(&mut state, &style, &mut zen_window, &mut perf).unwrap();
    let popup_row = &zen_window.cells[zen_stack.popup.y as usize];
    let left = popup_row
        .iter()
        .position(|character| *character == '╭')
        .unwrap();
    let right = popup_row
        .iter()
        .position(|character| *character == '╮')
        .unwrap();
    assert_eq!(left, 12 + zen_stack.popup.x as usize);
    assert_eq!(right - left + 1, zen_stack.popup.outer_w() as usize);
    assert_eq!(
        zen_window.backgrounds[zen_stack.popup.y as usize][0],
        Some(style.zen_margin)
    );
    let cursor = zen_window.cursor.unwrap();
    assert!(cursor.visible && (12..108).contains(&cursor.x));
    state.zen.enabled = false;

    let mut cramped_window = TestWindow::new(80, 8);
    draw_buffer_view(&mut state, &style, &mut cramped_window, &mut perf)
        .expect("cramped draw should succeed");
    assert!((0..8).any(|row| cramped_window.row_text(row).contains("Command")));

    state.apply_input(InputAction::CommandCancel, 80, 24);
    let expanded = popup_stack_layout(&state, &style, &window, (inner_w, inner_h));
    assert_eq!(expanded.popup.inner_h, inner_h);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn runtime_config_reload_applies_valid_changes_and_rejects_invalid_ones() {
    let dir = temp_dir_path("config_reload");
    fs::create_dir(&dir).expect("failed to create temp dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r##"
theme = "live"
text_formatting = false
leader = ","
icons_enabled = true
line_numbers = "absolute"

[animations]
yank_ripple_ms = 150
jump_pulse_ms = 275
undo_redo_ms = 200
dashboard_logo_ms = 0

[zen]
width_percent = 70
hide_gutter = false

[keybindings.normal]
open_finder = "<leader>f"
undo = "<ctrl-g>"

[keybindings.insert]
completion = "<ctrl-g>"

[themes.live.palette]
background = "#010203"
"##,
    )
    .expect("failed to write valid config");
    let launch_config = LaunchConfig {
        path: Some(config_path.clone()),
        ..LaunchConfig::default()
    };

    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);
    let mut keyboard = KeyboardHandler::new();
    let mut style = UiStyle::default();
    let mut active_config = crate::config::Config::default();
    let mut active_theme = active_config.theme.clone();
    let mut theme_override = None;
    state.zen.enabled = true;
    state.request_config_reload();
    reload_runtime_config(
        &mut state,
        &mut keyboard,
        &mut style,
        &mut active_config,
        &mut active_theme,
        &mut theme_override,
        &launch_config,
    );

    assert_eq!(style.theme.bg, Color::Rgb { r: 1, g: 2, b: 3 });
    assert!(state.zen.enabled);
    assert_eq!(state.zen.width_percent, 70);
    assert!(!state.zen.hide_gutter);
    assert!(style.icons_enabled);
    assert!(state.icons_enabled);
    assert!(!style.text_formatting);
    assert_eq!(state.animations.jump_highlight_ms, 275);
    assert_eq!(state.animations.undo_redo_highlight_ms, 200);
    assert_eq!(state.animations.dashboard_logo_ms, 0);
    assert_eq!(state.animations.yank_highlight_ms, 150);
    assert!(!style.about.title.format.bold);
    assert_eq!(
        style.layout.line_numbers,
        crate::ui::style::LineNumbers::Absolute
    );
    assert!(
        keyboard
            .keybinds()
            .values()
            .any(|action| { action == &KeybindAction::Custom("redox-key:<ctrl-g>".to_string()) })
    );
    assert_eq!(
        map_event_with_context(
            &mut state.input,
            crate::mode::EditorMode::Normal,
            false,
            &Event::Character(','),
        ),
        InputAction::None
    );
    assert_eq!(
        map_event_with_context(
            &mut state.input,
            crate::mode::EditorMode::Normal,
            false,
            &Event::Character('f'),
        ),
        InputAction::OpenFinder
    );

    fs::write(
        &config_path,
        "[logging]\nenabled = true\nmax_events = 1\n\
         [keybindings.normal]\nundo = \"<invalid-key>\"\n",
    )
    .expect("failed to write invalid config");
    let previous_style = style;
    let previous_config = format!("{active_config:?}");
    state.request_config_reload();
    reload_runtime_config(
        &mut state,
        &mut keyboard,
        &mut style,
        &mut active_config,
        &mut active_theme,
        &mut theme_override,
        &launch_config,
    );

    assert_eq!(style.theme, previous_style.theme);
    assert_eq!(
        style.layout.line_numbers,
        previous_style.layout.line_numbers
    );
    assert_eq!(format!("{active_config:?}"), previous_config);
    assert!(
        state
            .status_msg
            .as_deref()
            .is_some_and(|message| message.starts_with("configuration reload failed:"))
    );
    state.apply_input(InputAction::RunCommand("log reload check".into()), 80, 24);
    assert!(
        state
            .status_msg
            .as_deref()
            .is_some_and(|message| message.starts_with("logging is disabled;"))
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn config_command_opens_explicit_path_and_creates_its_parent() {
    let dir = temp_dir_path("config_open");
    let config_path = dir.join("nested/config.toml");
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);

    state.request_config_open();
    open_runtime_config(&mut state, Some(&config_path));

    let expected_path = config_path
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join("config.toml");
    assert_eq!(
        state.session.active_meta().path.as_deref(),
        Some(expected_path.as_path())
    );
    assert!(config_path.parent().is_some_and(std::path::Path::is_dir));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn colorscheme_command_applies_named_theme_for_current_session() {
    let config: crate::config::Config = toml::from_str(
        r##"
[themes.vague.palette]
background = "#141415"
"##,
    )
    .unwrap();
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);
    let mut style = config.style().unwrap();
    let mut active_theme = config.theme.clone();
    let mut theme_override = None;

    state.request_colorscheme("vague");
    apply_runtime_colorscheme(
        &mut state,
        &mut style,
        &config,
        &mut active_theme,
        &mut theme_override,
    );
    assert_eq!(active_theme, "vague");
    assert_eq!(theme_override.as_deref(), Some("vague"));
    assert_eq!(
        style.theme.bg,
        Color::Rgb {
            r: 20,
            g: 20,
            b: 21
        }
    );

    state.request_colorscheme("");
    apply_runtime_colorscheme(
        &mut state,
        &mut style,
        &config,
        &mut active_theme,
        &mut theme_override,
    );
    assert_eq!(state.status_msg.as_deref(), Some("colorscheme: vague"));

    state.request_colorscheme("missing");
    apply_runtime_colorscheme(
        &mut state,
        &mut style,
        &config,
        &mut active_theme,
        &mut theme_override,
    );
    assert_eq!(active_theme, "vague");
    assert_eq!(
        state.status_msg.as_deref(),
        Some("unknown colorscheme \"missing\"")
    );
}

#[test]
fn config_reload_preserves_session_colorscheme_and_refreshes_completions() {
    let dir = temp_dir_path("colorscheme_reload");
    fs::create_dir(&dir).unwrap();
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        "[themes.vague.palette]\nbackground = \"#141415\"\n",
    )
    .unwrap();
    let launch_config = LaunchConfig {
        path: Some(config_path.clone()),
        ..LaunchConfig::default()
    };
    let (mut config, _) = crate::config::Config::load(Some(&config_path)).unwrap();
    let session = EditorSession::open_initial_unnamed().expect("failed to open session");
    let mut state = EditorState::new(session);
    let mut keyboard = KeyboardHandler::new();
    let mut style = config.style().unwrap();
    let mut active_theme = config.theme.clone();
    let mut theme_override = None;
    state.request_colorscheme("vague");
    apply_runtime_colorscheme(
        &mut state,
        &mut style,
        &config,
        &mut active_theme,
        &mut theme_override,
    );

    fs::write(
        &config_path,
        r##"
[themes.default]
[themes.vague.palette]
background = "#202122"
[[bind]]
mode = "normal"
keys = " m"
command = " :e notes.md "
desc = "Open notes"
[[bind]]
mode = "normal"
keys = " n"
command = "e notes.md"
desc = "Open notes again"
"##,
    )
    .unwrap();
    state.request_config_reload();
    reload_runtime_config(
        &mut state,
        &mut keyboard,
        &mut style,
        &mut config,
        &mut active_theme,
        &mut theme_override,
        &launch_config,
    );

    assert_eq!(active_theme, "vague");
    assert_eq!(theme_override.as_deref(), Some("vague"));
    assert_eq!(
        style.theme.bg,
        Color::Rgb {
            r: 32,
            g: 33,
            b: 34
        }
    );
    for (prefix, expected) in [("colorscheme ", "default"), ("e notes", ".md")] {
        state.apply_input(InputAction::EnterCommand, 80, 24);
        for character in prefix.chars() {
            state.apply_input(InputAction::CommandChar(character), 80, 24);
        }
        assert_eq!(state.command_completion_suffix(), Some(expected));
        state.apply_input(InputAction::CommandCompletionNext, 80, 24);
        if prefix == "colorscheme " {
            assert_eq!(state.command_completion_suffix(), Some("vague"));
            state.apply_input(InputAction::CommandCompletionNext, 80, 24);
        }
        // Duplicate bindings and a configured default theme occur only once.
        assert_eq!(state.command_completion_suffix(), Some(expected));
        state.apply_input(InputAction::CommandComplete, 80, 24);
        assert_eq!(state.command_line, format!("{prefix}{expected}"));
    }

    fs::write(&config_path, "[themes.\"étoile\"]\n").unwrap();
    state.request_config_reload();
    reload_runtime_config(
        &mut state,
        &mut keyboard,
        &mut style,
        &mut config,
        &mut active_theme,
        &mut theme_override,
        &launch_config,
    );
    for (prefix, expected) in [
        ("colorscheme v", None),
        ("e notes", None),
        ("colorscheme é", Some("toile")),
    ] {
        state.apply_input(InputAction::EnterCommand, 80, 24);
        for character in prefix.chars() {
            state.apply_input(InputAction::CommandChar(character), 80, 24);
        }
        assert_eq!(state.command_completion_suffix(), expected);
    }
    state.apply_input(InputAction::CommandComplete, 80, 24);
    assert_eq!(state.command_line, "colorscheme étoile");
    let _ = fs::remove_dir_all(dir);
}
