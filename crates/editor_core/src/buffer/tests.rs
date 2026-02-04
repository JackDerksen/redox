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
    assert_eq!(end, Pos::new(0, 11));
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
