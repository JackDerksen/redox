use std::cmp::Ordering;
use std::fs;
use std::io::Write;

use tempfile::{NamedTempFile, tempdir};

use crate::motion::{Motion, apply_motion, apply_motion_for_operator, apply_motion_n};
use crate::{
    BufferKind, BufferLoadPhase, DelimiterKind, Edit, EditorSession, FuzzyQuery, Pos, Selection,
    TextBuffer, TextObjectKind, TextObjectScope, TextObjectSpec, UndoHistory, VisualModeKind,
    compare_path_match_scores, fuzzy_match_ranges, path_match_score,
};

#[test]
fn buffer_translates_character_byte_and_line_coordinates() {
    let buffer = TextBuffer::from_text("aé🙂\nβ");

    assert_eq!(buffer.len_chars(), 5);
    assert_eq!(buffer.len_bytes(), 10);
    assert_eq!(buffer.len_lines(), 2);
    assert_eq!(buffer.char(2), Some('🙂'));
    assert_eq!(buffer.char(5), None);
    assert_eq!(buffer.char_to_byte(3), 7);
    assert_eq!(buffer.byte_to_char(7), Some(3));
    assert_eq!(buffer.byte_to_char(11), None);
    assert_eq!(buffer.pos_to_char(Pos::new(1, 1)), 5);
    assert_eq!(buffer.char_to_pos(5), Pos::new(1, 1));
    assert_eq!(buffer.clamp_pos(Pos::new(99, 99)), Pos::new(1, 1));
}

#[test]
fn borrowed_slices_are_clamped_non_allocating_views() {
    let buffer = TextBuffer::from("alpha\nβeta\n");
    let slice = buffer.slice_chars_ref(10_000, 6);

    assert_eq!(String::from(slice), "βeta\n");
    assert_eq!(slice.len_chars(), 5);
    assert_eq!(slice.len_bytes(), 6);
    assert_eq!(buffer.line_slice(0).as_str(), Some("alpha"));
    assert_eq!(buffer.chars(1..4).collect::<String>(), "lph");
    assert_eq!(buffer.chunks().collect::<String>(), String::from(&buffer));

    let mut encoded = Vec::new();
    buffer.write_to(&mut encoded).expect("write buffer");
    let decoded = TextBuffer::from_reader(encoded.as_slice()).expect("read buffer");
    assert_eq!(decoded, buffer);
}

#[test]
fn primitive_edits_are_clamped_and_unicode_safe() {
    let mut buffer = TextBuffer::from_text("aé\nz");

    assert_eq!(buffer.insert(Pos::new(0, 2), "🙂"), Pos::new(0, 3));
    assert_eq!(buffer.to_string(), "aé🙂\nz");
    assert_eq!(
        buffer.backspace(Selection::empty(Pos::new(0, 3))),
        Selection::empty(Pos::new(0, 2))
    );
    assert_eq!(buffer.to_string(), "aé\nz");

    let selection = Selection::new(Pos::new(0, 0), Pos::new(0, 2));
    assert_eq!(
        buffer.replace_selection(selection, "Ω").cursor,
        Pos::new(0, 1)
    );
    assert_eq!(buffer.to_string(), "Ω\nz");

    let beyond_end = buffer.len_chars() + 99;
    let cursor = buffer.apply_edit(Edit::replace(beyond_end..0, "done"));
    assert_eq!(buffer.to_string(), "done");
    assert_eq!(cursor, Pos::new(0, 4));
}

#[test]
fn edit_batches_apply_sequentially_and_report_the_changed_region() {
    let mut buffer = TextBuffer::from_text("abcdef");
    let summary = buffer.apply_edits(&[
        Edit::replace(1..3, "X"),
        Edit::insert(4, "!"),
        Edit::delete(0..1),
    ]);

    assert_eq!(buffer.to_string(), "Xde!f");
    assert_eq!(summary.changed_range, 0..5);
    assert_eq!(summary.cursor, Pos::new(0, 0));
    assert_eq!(summary.edits_applied, 3);
}

#[test]
fn line_cleanup_and_indentation_are_policy_driven() {
    let mut buffer = TextBuffer::from_text("a  \n b\t\nc");

    assert!(buffer.trim_trailing_whitespace());
    assert_eq!(buffer.to_string(), "a\n b\nc");
    assert_eq!(buffer.indent_line_span(0, 1, 1, "  "), vec![(0, 2), (1, 2)]);
    assert_eq!(buffer.to_string(), "  a\n   b\nc");
    assert_eq!(buffer.outdent_line_span(0, 1, 1, 2), vec![(0, 2), (1, 2)]);
    assert_eq!(buffer.to_string(), "a\n b\nc");
    assert_eq!(buffer.replace_line_indent(1, "\t"), Some((1, 1)));
    assert_eq!(buffer.to_string(), "a\n\tb\nc");
}

#[test]
fn contiguous_line_ranges_move_without_losing_boundaries() {
    let mut buffer = TextBuffer::from_text("a\nb\nc\nd");

    assert_eq!(buffer.move_line_range_up(1, 2, 1), Some((0, 1)));
    assert_eq!(buffer.to_string(), "b\nc\na\nd");
    assert_eq!(buffer.move_line_range_down(0, 1, 2), Some((2, 3)));
    assert_eq!(buffer.to_string(), "a\nd\nb\nc");
}

#[test]
fn visual_selections_share_inclusive_editor_semantics() {
    let buffer = TextBuffer::from_text("abcd\nxy\nwxyz\n");
    let selection = Selection::new(Pos::new(0, 1), Pos::new(2, 2));

    assert_eq!(
        buffer.visual_selection_text(selection, VisualModeKind::Char),
        "bcd\nxy\nwxy"
    );
    assert_eq!(
        buffer.visual_selection_text(selection, VisualModeKind::Line),
        "abcd\nxy\nwxyz\n"
    );
    assert_eq!(
        buffer.visual_selection_text(selection, VisualModeKind::Block),
        "bc\ny\nxy"
    );
    assert_eq!(
        buffer
            .visual_selection_edit_plan(selection, VisualModeKind::Block)
            .delete_ranges,
        vec![
            (Pos::new(0, 1), Pos::new(0, 3)),
            (Pos::new(1, 1), Pos::new(1, 2)),
            (Pos::new(2, 1), Pos::new(2, 3)),
        ]
    );
}

#[test]
fn literal_search_handles_unicode_and_rope_chunk_boundaries() {
    let needle = "α🙂β";
    let source = format!("{}{}{}", "x".repeat(100_000), needle, "x".repeat(100_000));
    let buffer = TextBuffer::from_text(&source);

    assert!(buffer.chunks().count() > 1);
    assert_eq!(
        buffer.find_matches(needle),
        vec![(Pos::new(0, 100_000), Pos::new(0, 100_003))]
    );
    assert_eq!(
        TextBuffer::from_text("aaaa").find_matches("aa"),
        vec![
            (Pos::new(0, 0), Pos::new(0, 2)),
            (Pos::new(0, 2), Pos::new(0, 4)),
        ]
    );
}

#[test]
fn character_search_and_delimiter_matching_stay_structural() {
    let buffer = TextBuffer::from_text(r#"(alpha \( beta) "quoted \" text""#);

    assert_eq!(
        buffer.find_char_after_on_line(Pos::zero(), 'b'),
        Some(Pos::new(0, 10))
    );
    assert_eq!(
        buffer.find_char_before_on_line(Pos::new(0, 14), 'a'),
        Some(Pos::new(0, 13))
    );
    assert_eq!(
        buffer.matching_delimiter(Pos::zero()),
        Some(Pos::new(0, 14))
    );
    assert_eq!(buffer.matching_delimiter(Pos::new(0, 7)), None);
    assert_eq!(
        buffer.matching_delimiter(Pos::new(0, 16)),
        Some(Pos::new(0, 31))
    );
}

#[test]
fn text_objects_resolve_words_delimiters_and_paragraphs() {
    let buffer = TextBuffer::from_text("alpha (beta gamma)\n\nsecond paragraph\n");

    let word = buffer
        .text_object_edit_plan(
            Pos::new(0, 8),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            },
        )
        .expect("word object");
    assert_eq!(word.text, "beta");

    let delimiters = buffer
        .text_object_edit_plan(
            Pos::new(0, 10),
            TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Delimiter(DelimiterKind::Parentheses),
                count: 1,
            },
        )
        .expect("delimiter object");
    assert_eq!(delimiters.text, "(beta gamma)");

    let paragraph = buffer
        .text_object_edit_plan(
            Pos::new(2, 3),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Paragraph,
                count: 1,
            },
        )
        .expect("paragraph object");
    assert_eq!(paragraph.mode, VisualModeKind::Line);
    assert_eq!(paragraph.text, "second paragraph\n");
}

#[test]
fn motions_compose_counts_and_operator_ranges() {
    let buffer = TextBuffer::from_text("alpha beta\nshort\nlonger line\n");

    assert_eq!(
        apply_motion_n(&buffer, Pos::zero(), Motion::WordStartAfter, 2),
        Pos::new(1, 0)
    );
    assert_eq!(
        apply_motion(&buffer, Pos::new(2, 8), Motion::Up),
        Pos::new(1, 5)
    );
    assert_eq!(
        apply_motion_for_operator(&buffer, Pos::zero(), Motion::FindChar('b'), 1),
        Pos::new(0, 7)
    );
    assert_eq!(
        apply_motion(&buffer, Pos::new(0, 99), Motion::LineEnd),
        Pos::new(0, 10)
    );
}

#[test]
fn undo_history_round_trips_and_preserves_branches() {
    let mut history = UndoHistory::default();
    let mut buffer = TextBuffer::from_text("a");

    let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 1));
    buffer.insert(Pos::new(0, 1), "b");
    assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));
    let first_branch = history.current();

    assert_eq!(history.undo(&mut buffer), Some(Pos::new(0, 1)));
    let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 1));
    buffer.insert(Pos::new(0, 1), "c");
    assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));

    assert_eq!(buffer.to_string(), "ac");
    assert_eq!(
        history.restore(&mut buffer, first_branch),
        Some(Pos::new(0, 2))
    );
    assert_eq!(buffer.to_string(), "ab");
    assert_eq!(history.tree_entries()[0].child_count, 2);
}

#[test]
fn fuzzy_ranking_prefers_contiguous_filename_matches() {
    let query = FuzzyQuery::new("state");
    let direct_label = "crates/redox-tui/src/app/state.rs";
    let indirect_label = "stateful/src/app/main.rs";
    let direct_match = fuzzy_match_ranges(direct_label, &query).expect("direct match");
    let indirect_match = fuzzy_match_ranges(indirect_label, &query).expect("indirect match");

    assert_eq!(
        compare_path_match_scores(
            &path_match_score(direct_label, &direct_match, &query),
            &path_match_score(indirect_label, &indirect_match, &query),
        ),
        Ordering::Less
    );
    assert_eq!(direct_match.highlights, vec![25..30]);
}

#[test]
fn sessions_manage_multiple_buffers_and_dirty_state() {
    let dir = tempdir().expect("temp directory");
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    fs::write(&first, "first").expect("first fixture");
    fs::write(&second, "second").expect("second fixture");

    let mut session = EditorSession::open_initial_file(&first).expect("initial session");
    let first_id = session.active_id();
    let second_id = session.open_file(&second).expect("second buffer");
    assert_ne!(first_id, second_id);
    assert_eq!(session.switch_next_mru(), Some(first_id));

    session.active_buffer_mut().insert(Pos::new(0, 5), "!");
    assert!(session.recompute_active_dirty());
    assert!(session.any_dirty());
    assert_eq!(session.active_meta().kind, BufferKind::File);
    assert_eq!(session.summaries().len(), 2);
}

#[test]
fn saving_normalises_the_final_newline_and_rejects_external_changes() {
    let mut file = NamedTempFile::new().expect("temp file");
    file.write_all(b"hello").expect("fixture write");
    file.flush().expect("fixture flush");

    let mut session = EditorSession::open_initial_file(file.path()).expect("session");
    session.save_active().expect("first save");
    assert_eq!(
        fs::read_to_string(file.path()).expect("saved text"),
        "hello\n"
    );

    session.active_buffer_mut().insert(Pos::new(0, 5), " local");
    assert!(session.recompute_active_dirty());
    fs::write(file.path(), "changed somewhere else\n").expect("external write");
    assert!(session.save_active().is_err());
    assert!(session.active_meta().external_changed);
}

#[test]
fn incremental_loading_preserves_unicode_across_chunk_boundaries() {
    let mut file = NamedTempFile::new().expect("temp file");
    let text = "😀alpha\nβeta\nこんにちは\n".repeat(7_000);
    file.write_all(text.as_bytes()).expect("fixture write");
    file.flush().expect("fixture flush");

    let mut session = EditorSession::open_initial_file(file.path()).expect("session");
    assert_eq!(
        session.active_buffer_load_status().phase,
        BufferLoadPhase::Loading
    );
    let id = session.active_id();
    session.ensure_buffer_fully_loaded(id).expect("full load");

    assert_eq!(
        session.active_buffer_load_status().phase,
        BufferLoadPhase::Complete
    );
    assert_eq!(session.active_buffer().to_string(), text);
    assert!(!session.recompute_active_dirty());
}

#[test]
fn incremental_loading_rejects_invalid_utf8() {
    let mut file = NamedTempFile::new().expect("temp file");
    file.write_all("valid\n".repeat(20_000).as_bytes())
        .expect("valid prefix");
    file.write_all(&[0xff]).expect("invalid suffix");
    file.flush().expect("fixture flush");

    let mut session = EditorSession::open_initial_file(file.path()).expect("session");
    let id = session.active_id();
    let error = session
        .ensure_buffer_fully_loaded(id)
        .expect_err("invalid UTF-8");

    assert!(error.to_string().contains("not valid UTF-8"));
    assert_eq!(
        session.active_buffer_load_status().phase,
        BufferLoadPhase::Failed
    );
}

#[test]
fn path_reconciliation_remaps_clean_files_and_orphans_dirty_deletions() {
    let root = tempdir().expect("temp directory");
    let old_dir = root.path().join("old");
    let new_dir = root.path().join("new");
    fs::create_dir(&old_dir).expect("old directory");
    let clean_path = old_dir.join("clean.txt");
    let dirty_path = old_dir.join("dirty.txt");
    fs::write(&clean_path, "clean").expect("clean fixture");
    fs::write(&dirty_path, "dirty").expect("dirty fixture");

    let mut session = EditorSession::open_initial_file(&clean_path).expect("session");
    let clean_id = session.active_id();
    let dirty_id = session.open_file(&dirty_path).expect("dirty buffer");
    session.active_buffer_mut().insert(Pos::new(0, 5), "!");
    assert!(session.recompute_active_dirty());

    fs::rename(&old_dir, &new_dir).expect("directory rename");
    let renamed = session.sync_file_buffers_with_paths(&[(old_dir, new_dir.clone())], &[]);
    assert_eq!(renamed.remapped_ids.len(), 2);
    assert!(renamed.remapped_ids.contains(&clean_id));
    assert!(renamed.remapped_ids.contains(&dirty_id));

    let renamed_dirty = new_dir.join("dirty.txt");
    fs::remove_file(&renamed_dirty).expect("dirty deletion");
    let deleted = session.sync_file_buffers_with_paths(&[], &[renamed_dirty]);
    assert!(deleted.closed_ids.is_empty());
    let meta = session.meta(dirty_id).expect("orphaned buffer");
    assert!(meta.path.is_none());
    assert!(meta.dirty);
    assert!(meta.display_name.ends_with(" [orphaned]"));
}
