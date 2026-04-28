//! Unit tests for the rope-backed buffer live here to keep the main modules smaller.

use super::*;

#[test]
fn pos_char_roundtrip_basic() {
    let b = TextBuffer::from_str("hello\nworld\n");
    // line 0: "hello"
    let p = Pos::new(0, 3);
    let c = b.pos_to_char(p);
    assert_eq!(b.rope().char(c), 'l');
    let p2 = b.char_to_pos(c);
    assert_eq!(p2, p);
}

#[test]
fn clamp_column_to_line_len() {
    let b = TextBuffer::from_str("hi\n");
    let p = b.clamp_pos(Pos::new(0, 999));
    assert_eq!(p, Pos::new(0, 2));
}

#[test]
fn insert_and_delete() {
    let mut b = TextBuffer::from_str("ac\n");
    let cur = b.insert(Pos::new(0, 1), "b");
    assert_eq!(b.to_string(), "abc\n");
    assert_eq!(cur, Pos::new(0, 2));

    let cur2 = b.delete_range(Pos::new(0, 1), Pos::new(0, 2));
    assert_eq!(b.to_string(), "ac\n");
    assert_eq!(cur2, Pos::new(0, 1));
}

#[test]
fn backspace_at_start_noop() {
    let mut b = TextBuffer::from_str("x");
    let sel = Selection::empty(Pos::new(0, 0));
    let sel2 = b.backspace(sel);
    assert_eq!(b.to_string(), "x");
    assert_eq!(sel2.cursor, Pos::new(0, 0));
}

#[test]
fn delete_forward() {
    let mut b = TextBuffer::from_str("xy");
    let sel = Selection::empty(Pos::new(0, 0));
    let sel2 = b.delete(sel);
    assert_eq!(b.to_string(), "y");
    assert_eq!(sel2.cursor, Pos::new(0, 0));
}

#[test]
fn selection_slice_and_delete() {
    let mut b = TextBuffer::from_str("hello world");
    let sel = Selection::new(Pos::new(0, 6), Pos::new(0, 11));
    assert_eq!(b.slice_selection(sel), "world");
    let (cur, did) = b.delete_selection(sel);
    assert!(did);
    assert_eq!(cur, Pos::new(0, 6));
    assert_eq!(b.to_string(), "hello ");
}

#[test]
fn word_motions_ascii() {
    let b = TextBuffer::from_str("abc  def_12!");
    let p = Pos::new(0, 6); // in "def_12"
    let start = b.word_start_before(p);
    assert_eq!(start, Pos::new(0, 5));
    let end = b.word_end_after(start);
    assert_eq!(end, Pos::new(0, 10));

    let symbol = b.word_start_after(start);
    assert_eq!(symbol, Pos::new(0, 11));

    let back_to_symbol = b.word_start_before(Pos::new(0, 12));
    assert_eq!(back_to_symbol, Pos::new(0, 11));
}

#[test]
fn word_motions_visit_symbols() {
    let b = TextBuffer::from_str("(normal/insert/command)");

    let p1 = b.word_start_after(Pos::new(0, 0));
    assert_eq!(p1, Pos::new(0, 1)); // normal

    let p2 = b.word_start_after(p1);
    assert_eq!(p2, Pos::new(0, 7)); // /

    let p3 = b.word_start_after(p2);
    assert_eq!(p3, Pos::new(0, 8)); // insert

    let p4 = b.word_start_after(p3);
    assert_eq!(p4, Pos::new(0, 14)); // /

    let p5 = b.word_start_after(p4);
    assert_eq!(p5, Pos::new(0, 15)); // command

    let p6 = b.word_start_after(p5);
    assert_eq!(p6, Pos::new(0, 22)); // )
}

#[test]
fn line_len_excludes_newline() {
    let b = TextBuffer::from_str("a\nbb\n");
    assert_eq!(b.line_len_chars(0), 1);
    assert_eq!(b.line_len_chars(1), 2);
}

#[test]
fn line_range_excludes_newline() {
    let b = TextBuffer::from_str("a\nbb\n");
    let r0 = b.line_char_range(0);
    assert_eq!(b.slice_chars(r0.start, r0.end), "a");
    let r1 = b.line_char_range(1);
    assert_eq!(b.slice_chars(r1.start, r1.end), "bb");
}

#[test]
fn apply_edit_replace() {
    let mut b = TextBuffer::from_str("kitten");
    // "kit" -> "smit"
    let cur = b.apply_edit(Edit::replace(0..3, "smit"));
    assert_eq!(b.to_string(), "smitten");
    assert_eq!(cur, Pos::new(0, 4));
}

#[test]
fn slice_chars_clamps_and_swaps_indices() {
    let b = TextBuffer::from_str("abcdef");
    assert_eq!(b.slice_chars(2, 5), "cde");
    assert_eq!(b.slice_chars(5, 2), "cde");
    assert_eq!(b.slice_chars(0, 999), "abcdef");
}

#[test]
fn slice_pos_range_is_order_independent() {
    let b = TextBuffer::from_str("hello world");
    let a = Pos::new(0, 1);
    let bpos = Pos::new(0, 5);
    assert_eq!(b.slice_pos_range(a, bpos), "ello");
    assert_eq!(b.slice_pos_range(bpos, a), "ello");
}

#[test]
fn replace_selection_replaces_non_empty_range() {
    let mut b = TextBuffer::from_str("hello world");
    let sel = Selection::new(Pos::new(0, 6), Pos::new(0, 11));
    let out = b.replace_selection(sel, "rust");
    assert_eq!(out, Selection::empty(Pos::new(0, 10)));
    assert_eq!(b.to_string(), "hello rust");
}

#[test]
fn insert_newline_replaces_selection() {
    let mut b = TextBuffer::from_str("ab");
    let sel = Selection::new(Pos::new(0, 0), Pos::new(0, 2));
    let out = b.insert_newline(sel);
    assert_eq!(out.cursor, Pos::new(1, 0));
    assert_eq!(b.to_string(), "\n");
}

#[test]
fn delete_forward_at_eof_is_noop() {
    let mut b = TextBuffer::from_str("abc");
    let sel = Selection::empty(Pos::new(0, 3));
    let out = b.delete(sel);
    assert_eq!(out.cursor, Pos::new(0, 3));
    assert_eq!(b.to_string(), "abc");
}

#[test]
fn char_queries_handle_line_end_and_start() {
    let b = TextBuffer::from_str("ab\n");
    assert_eq!(b.char_at(Pos::new(0, 0)), Some('a'));
    assert_eq!(b.char_at(Pos::new(0, 2)), None);
    assert_eq!(b.char_before(Pos::new(0, 0)), None);
    assert_eq!(b.char_before(Pos::new(0, 1)), Some('a'));
}

#[test]
fn line_and_char_index_conversion_clamps() {
    let b = TextBuffer::from_str("a\nbb\n");
    assert_eq!(b.line_to_char(999), b.line_to_char(2));
    assert_eq!(b.char_to_line(999), 2);
}

#[test]
fn apply_edit_normalizes_reversed_range() {
    let mut b = TextBuffer::from_str("abcdef");
    let cur = b.apply_edit(Edit::replace(std::ops::Range { start: 4, end: 2 }, "XY"));
    assert_eq!(b.to_string(), "abXYef");
    assert_eq!(cur, Pos::new(0, 4));
}

#[test]
fn slice_chars_ref_matches_allocating_variant() {
    let b = TextBuffer::from_str("abcdef");
    assert_eq!(b.slice_chars_ref(2, 5).to_string(), b.slice_chars(2, 5));
    assert_eq!(b.slice_chars_ref(5, 2).to_string(), b.slice_chars(5, 2));
}

#[test]
fn line_slice_excludes_trailing_newline() {
    let b = TextBuffer::from_str("abc\ndef\n");
    assert_eq!(b.line_slice(0).to_string(), "abc");
    assert_eq!(b.line_slice(1).to_string(), "def");
    assert_eq!(b.line_slice(2).to_string(), "");
}

#[test]
fn apply_edits_returns_summary_and_final_cursor() {
    let mut b = TextBuffer::from_str("hello world");
    let edits = vec![Edit::replace(6..11, "rust"), Edit::insert(10, "!")];
    let summary = b.apply_edits(&edits);

    assert_eq!(b.to_string(), "hello rust!");
    assert_eq!(summary.cursor, Pos::new(0, 11));
    assert_eq!(summary.edits_applied, 2);
    assert_eq!(summary.changed_range, 6..11);
}

#[test]
fn text_object_inner_word_selects_current_word() {
    let b = TextBuffer::from_str("alpha beta");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 7),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            },
        )
        .expect("word text object");

    assert_eq!(plan.text, "beta");
    assert_eq!(plan.mode, VisualModeKind::Char);
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 6), Pos::new(0, 10))]);
}

#[test]
fn text_object_around_word_includes_spacing() {
    let b = TextBuffer::from_str("alpha beta gamma");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 7),
            TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Word,
                count: 1,
            },
        )
        .expect("word text object");

    assert_eq!(plan.text, "beta ");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 6), Pos::new(0, 11))]);
}

#[test]
fn text_object_inner_word_on_whitespace_prefers_next_word() {
    let b = TextBuffer::from_str("alpha  beta");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 5),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            },
        )
        .expect("word text object");

    assert_eq!(plan.text, "beta");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 7), Pos::new(0, 11))]);
}

#[test]
fn text_object_inner_big_word_selects_contiguous_non_whitespace() {
    let b = TextBuffer::from_str("byte_idx.saturating_add(grapheme.len()); next");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 22),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::BigWord,
                count: 1,
            },
        )
        .expect("big word text object");

    assert_eq!(plan.text, "byte_idx.saturating_add(grapheme.len());");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 0), Pos::new(0, 40))]);
}

#[test]
fn counted_text_object_word_clamps_at_last_run_instead_of_failing() {
    let b = TextBuffer::from_str("alpha beta");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 1),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 3,
            },
        )
        .expect("counted word text object");

    assert_eq!(plan.text, "alpha beta");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 0), Pos::new(0, 10))]);
}

#[test]
fn text_object_selection_uses_word_bounds_for_visual_mode() {
    let b = TextBuffer::from_str("alpha beta");
    let (selection, mode) = b
        .text_object_selection(
            Pos::new(0, 6),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Word,
                count: 1,
            },
        )
        .expect("word selection");

    assert_eq!(mode, VisualModeKind::Char);
    assert_eq!(selection.anchor, Pos::new(0, 6));
    assert_eq!(selection.cursor, Pos::new(0, 9));
}

#[test]
fn text_object_inner_paragraph_is_linewise() {
    let b = TextBuffer::from_str("one\ntwo\n\nthree\n");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 1),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Paragraph,
                count: 1,
            },
        )
        .expect("paragraph text object");

    assert_eq!(plan.text, "one\ntwo\n");
    assert_eq!(plan.mode, VisualModeKind::Line);
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 0), Pos::new(2, 0))]);
}

#[test]
fn text_object_around_brackets_selects_pair() {
    let b = TextBuffer::from_str("foo[bar[baz]]");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 9),
            TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                count: 1,
            },
        )
        .expect("delimiter text object");

    assert_eq!(plan.text, "[baz]");
    assert_eq!(plan.mode, VisualModeKind::Char);
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 7), Pos::new(0, 12))]);
}

#[test]
fn text_object_inner_parentheses_can_select_outer_pair_with_count() {
    let b = TextBuffer::from_str("((abc))");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 3),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::Parentheses),
                count: 2,
            },
        )
        .expect("outer delimiter text object");

    assert_eq!(plan.text, "(abc)");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 1), Pos::new(0, 6))]);
}

#[test]
fn text_object_inner_parentheses_falls_back_to_same_line_pair_after_cursor() {
    let b = TextBuffer::from_str("call foo(bar) now");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 0),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::Parentheses),
                count: 1,
            },
        )
        .expect("same-line delimiter text object");

    assert_eq!(plan.text, "bar");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 9), Pos::new(0, 12))]);
}

#[test]
fn text_object_around_brackets_falls_back_to_same_line_pair_before_cursor() {
    let b = TextBuffer::from_str("left [mid] right");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 15),
            TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                count: 1,
            },
        )
        .expect("same-line delimiter text object");

    assert_eq!(plan.text, "[mid]");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 5), Pos::new(0, 10))]);
}

#[test]
fn text_object_inner_double_quotes_falls_back_to_same_line_pair() {
    let b = TextBuffer::from_str("say \"hello\" now");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 0),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes),
                count: 1,
            },
        )
        .expect("same-line quote text object");

    assert_eq!(plan.text, "hello");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 5), Pos::new(0, 10))]);
}

#[test]
fn text_object_around_single_quotes_skips_escaped_quotes() {
    let b = TextBuffer::from_str("'one' '\\''");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 9),
            TextObjectSpec {
                scope: TextObjectScope::Around,
                kind: TextObjectKind::Delimiter(DelimiterKind::SingleQuotes),
                count: 1,
            },
        )
        .expect("quote text object");

    assert_eq!(plan.text, "'\\''");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 6), Pos::new(0, 10))]);
}

#[test]
fn text_object_inner_backticks_selects_current_quoted_span() {
    let b = TextBuffer::from_str("run `cargo test` now");
    let plan = b
        .text_object_edit_plan(
            Pos::new(0, 10),
            TextObjectSpec {
                scope: TextObjectScope::Inner,
                kind: TextObjectKind::Delimiter(DelimiterKind::Backticks),
                count: 1,
            },
        )
        .expect("backtick text object");

    assert_eq!(plan.text, "cargo test");
    assert_eq!(plan.delete_ranges, vec![(Pos::new(0, 5), Pos::new(0, 15))]);
}

#[test]
fn apply_edits_empty_is_noop_summary() {
    let mut b = TextBuffer::from_str("abc");
    let summary = b.apply_edits(&[]);
    assert_eq!(b.to_string(), "abc");
    assert_eq!(summary.changed_range, 3..3);
    assert_eq!(summary.cursor, Pos::new(0, 3));
    assert_eq!(summary.edits_applied, 0);
}

#[test]
fn line_span_char_range_preserves_line_boundaries() {
    let b = TextBuffer::from_str("one\ntwo");
    let r0 = b.line_span_char_range(0, 0);
    assert_eq!(b.slice_chars(r0.start, r0.end), "one\n");

    let r1 = b.line_span_char_range(1, 1);
    assert_eq!(b.slice_chars(r1.start, r1.end), "two");
}

#[test]
fn line_span_text_linewise_register_always_has_trailing_newline() {
    let b = TextBuffer::from_str("one\ntwo");
    let text = b.line_span_text_linewise_register(1, 1);
    assert_eq!(text, "two\n");
}

#[test]
fn selection_line_range_is_order_independent() {
    let sel = Selection::new(Pos::new(3, 0), Pos::new(1, 9));
    assert_eq!(sel.line_range(), (1, 3));
}

#[test]
fn visual_charwise_range_is_inclusive_of_end_cursor() {
    let b = TextBuffer::from_str("abcd");
    let sel = Selection::new(Pos::new(0, 1), Pos::new(0, 2));
    assert_eq!(b.visual_charwise_text(sel), "bc");
}

#[test]
fn visual_linewise_text_uses_linewise_register_semantics() {
    let b = TextBuffer::from_str("one\ntwo");
    let sel = Selection::new(Pos::new(1, 0), Pos::new(1, 0));
    assert_eq!(b.visual_linewise_text(sel), "two\n");
}

#[test]
fn visual_selection_helpers_dispatch_by_visual_mode() {
    let b = TextBuffer::from_str("abcd\nef\n");
    let sel = Selection::new(Pos::new(0, 1), Pos::new(1, 0));

    assert_eq!(
        b.visual_selection_pos_ranges(sel, VisualModeKind::Char),
        vec![(Pos::new(0, 1), Pos::new(1, 1))]
    );
    assert_eq!(b.visual_selection_text(sel, VisualModeKind::Char), "bcd\ne");

    assert_eq!(
        b.visual_selection_pos_ranges(sel, VisualModeKind::Line),
        vec![(Pos::new(0, 0), Pos::new(2, 0))]
    );
    assert_eq!(
        b.visual_selection_text(sel, VisualModeKind::Line),
        "abcd\nef\n"
    );

    assert_eq!(
        b.visual_selection_pos_ranges(sel, VisualModeKind::Block),
        vec![
            (Pos::new(0, 0), Pos::new(0, 2)),
            (Pos::new(1, 0), Pos::new(2, 0))
        ]
    );
    assert_eq!(
        b.visual_selection_text(sel, VisualModeKind::Block),
        "ab\nef"
    );
}

#[test]
fn visual_selection_edit_plan_bundles_delete_bounds_and_text() {
    let b = TextBuffer::from_str("abcd\nef\n");
    let sel = Selection::new(Pos::new(0, 1), Pos::new(1, 0));

    let charwise = b.visual_selection_edit_plan(sel, VisualModeKind::Char);
    assert_eq!(
        charwise.delete_ranges,
        vec![(Pos::new(0, 1), Pos::new(1, 1))]
    );
    assert_eq!(charwise.text, "bcd\ne");
    assert_eq!(charwise.mode, VisualModeKind::Char);

    let linewise = b.visual_selection_edit_plan(sel, VisualModeKind::Line);
    assert_eq!(
        linewise.delete_ranges,
        vec![(Pos::new(0, 0), Pos::new(2, 0))]
    );
    assert_eq!(linewise.text, "abcd\nef\n");
    assert_eq!(linewise.mode, VisualModeKind::Line);

    let blockwise = b.visual_selection_edit_plan(sel, VisualModeKind::Block);
    assert_eq!(
        blockwise.delete_ranges,
        vec![
            (Pos::new(0, 0), Pos::new(0, 2)),
            (Pos::new(1, 0), Pos::new(2, 0))
        ]
    );
    assert_eq!(blockwise.text, "ab\nef");
    assert_eq!(blockwise.mode, VisualModeKind::Block);
}

#[test]
fn visual_selection_char_range_on_line_matches_visual_semantics() {
    let b = TextBuffer::from_str("abcd\nefgh\n");
    let sel = Selection::new(Pos::new(0, 1), Pos::new(1, 2));

    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Char, 0),
        Some(1..4)
    );
    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Char, 1),
        Some(0..3)
    );
    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Char, 2),
        None
    );

    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Line, 0),
        Some(0..4)
    );
    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Line, 1),
        Some(0..4)
    );

    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Block, 0),
        Some(1..3)
    );
    assert_eq!(
        b.visual_selection_char_range_on_line(sel, VisualModeKind::Block, 1),
        Some(1..3)
    );
}

#[test]
fn move_line_range_down_and_up_preserve_content() {
    let mut b = TextBuffer::from_str("one\ntwo\nthree\nfour\n");
    let moved = b.move_line_range_down_once(1, 2);
    assert_eq!(moved, Some((2, 3)));
    assert_eq!(b.to_string(), "one\nfour\ntwo\nthree\n");

    let moved_back = b.move_line_range_up_once(2, 3);
    assert_eq!(moved_back, Some((1, 2)));
    assert_eq!(b.to_string(), "one\ntwo\nthree\nfour\n");
}

#[test]
fn move_line_range_down_into_trailing_blank_line_preserves_split() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    let moved = b.move_line_range_down_once(1, 1);
    assert_eq!(moved, Some((2, 2)));
    assert_eq!(b.to_string(), "one\n\ntwo");
}

#[test]
fn move_line_range_up_at_eof_without_trailing_newline_does_not_join() {
    let mut b = TextBuffer::from_str("one\ntwo");
    let moved = b.move_line_range_up_once(1, 1);
    assert_eq!(moved, Some((0, 0)));
    assert_eq!(b.to_string(), "two\none");
}

#[test]
fn move_line_range_respects_boundaries() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    assert_eq!(b.move_line_range_up_once(0, 0), None);
    assert_eq!(b.move_line_range_down_once(2, 2), None);
    assert_eq!(b.to_string(), "one\ntwo\n");
}

#[test]
fn move_line_range_up_multi_step_stops_at_top() {
    let mut b = TextBuffer::from_str("one\ntwo\nthree\nfour\n");
    let moved = b.move_line_range_up(2, 2, 5);

    assert_eq!(moved, Some((0, 0)));
    assert_eq!(b.to_string(), "three\none\ntwo\nfour\n");
}

#[test]
fn move_line_range_down_multi_step_stops_at_bottom() {
    let mut b = TextBuffer::from_str("one\ntwo\nthree\nfour\n");
    let moved = b.move_line_range_down(0, 0, 5);

    assert_eq!(moved, Some((4, 4)));
    assert_eq!(b.to_string(), "two\nthree\nfour\n\none");
}

#[test]
fn move_line_range_multi_step_returns_none_when_unmoved() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    assert_eq!(b.move_line_range_up(0, 0, 1), None);
    assert_eq!(b.move_line_range_down(2, 2, 3), None);
}

#[test]
fn indent_line_span_adds_tabs_and_reports_counts() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    let added = b.indent_line_span(0, 1, 2);

    assert_eq!(added, vec![(0, 2), (1, 2)]);
    assert_eq!(b.to_string(), "\t\tone\n\t\ttwo\n");
}

#[test]
fn outdent_line_span_removes_tabs_and_spaces() {
    let mut b = TextBuffer::from_str("\t  one\n    two\nthree\n");
    let removed = b.outdent_line_span(0, 2, 1);

    assert_eq!(removed, vec![(0, 1), (1, 4), (2, 0)]);
    assert_eq!(b.to_string(), "  one\ntwo\nthree\n");
}

#[test]
fn paste_before_charwise_inserts_at_cursor_and_advances() {
    let mut b = TextBuffer::from_str("abcd");
    let new_cursor = b.paste_before(Pos::new(0, 1), "X", false);

    assert_eq!(b.to_string(), "aXbcd");
    assert_eq!(new_cursor, Pos::new(0, 2));
}

#[test]
fn paste_before_linewise_inserts_at_line_start_and_advances() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    let new_cursor = b.paste_before(Pos::new(1, 2), "zero\n", true);

    assert_eq!(b.to_string(), "one\nzero\ntwo\n");
    assert_eq!(new_cursor, Pos::new(1, 0));
}

#[test]
fn paste_after_charwise_inserts_after_cursor_and_clamps_at_eol() {
    let mut b = TextBuffer::from_str("abcd");
    let new_cursor = b.paste_after(Pos::new(0, 1), "X", false);
    assert_eq!(b.to_string(), "abXcd");
    assert_eq!(new_cursor, Pos::new(0, 3));

    let end_cursor = b.paste_after(Pos::new(0, 5), "!", false);
    assert_eq!(b.to_string(), "abXcd!");
    assert_eq!(end_cursor, Pos::new(0, 6));
}

#[test]
fn paste_after_linewise_inserts_at_next_line_start_and_advances() {
    let mut b = TextBuffer::from_str("one\ntwo\n");
    let new_cursor = b.paste_after(Pos::new(0, 1), "x\n", true);

    assert_eq!(b.to_string(), "one\nx\ntwo\n");
    assert_eq!(new_cursor, Pos::new(1, 0));
}
