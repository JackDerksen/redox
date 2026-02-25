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
    let cur = b.apply_edit(Edit::replace(4..2, "XY"));
    assert_eq!(b.to_string(), "abXYef");
    assert_eq!(cur, Pos::new(0, 4));
}
