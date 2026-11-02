//! Editing notes in the user's editor, the way `git commit` does.

use std::process::Command;

/// `$VISUAL`, else `$EDITOR`, else `vi` (the first one that is not blank).
pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> String {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("vi")
        .to_string()
}

/// Write `text` (plus a final newline) to a temporary `notes.md`, run `editor` on it through
/// `sh` with the terminal's stdio, and return the file's content with one trailing newline
/// removed. Err when the editor cannot start or exits unsuccessfully.
pub fn edit_text(editor: &str, text: &str) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("cannot create a temp file: {e}"))?;
    let path = dir.path().join("notes.md");
    std::fs::write(&path, format!("{text}\n")).map_err(|e| e.to_string())?;
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()
        .map_err(|e| format!("cannot run {editor}: {e}"))?;
    if !status.success() {
        return Err(format!("{editor} exited with {status}"));
    }
    let out = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(out.strip_suffix('\n').unwrap_or(&out).to_string())
}
