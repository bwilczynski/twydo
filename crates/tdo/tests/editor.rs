//! Notes editing through an external editor command.

use tdo::editor::{edit_text, editor_command};

#[test]
fn editor_command_precedence() {
    assert_eq!(editor_command(Some("hx"), Some("nvim")), "hx");
    assert_eq!(editor_command(Some(""), Some("nvim")), "nvim");
    assert_eq!(editor_command(None, Some("nvim")), "nvim");
    assert_eq!(editor_command(None, Some("  ")), "vi");
    assert_eq!(editor_command(None, None), "vi");
}

#[test]
fn edit_text_returns_new_content() {
    assert_eq!(edit_text("sed -i s/old/new/", "old"), Ok("new".into()));
}

#[test]
fn edit_text_strips_one_newline() {
    assert_eq!(
        edit_text("sh -c 'printf \"a\\n\\n\" > \"$0\"'", "x"),
        Ok("a\n".into())
    );
}

#[test]
fn edit_text_keeps_unicode_and_lines() {
    assert_eq!(
        edit_text("true", "zażółć\nline 2"),
        Ok("zażółć\nline 2".into())
    );
}

#[test]
fn failing_editor_is_an_error() {
    let err = edit_text("false", "keep").unwrap_err();
    assert!(err.contains("exited"), "{err}");
    let err = edit_text("/nonexistent/editor-xyz", "keep").unwrap_err();
    assert!(!err.is_empty());
}
