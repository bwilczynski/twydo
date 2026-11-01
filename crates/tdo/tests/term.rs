use std::time::Duration;

use tdo::term::watch;

#[test]
fn reload_after_file_change() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("list");
    std::fs::create_dir(&list).unwrap();
    let (_debouncer, rx) = watch(dir.path()).unwrap();

    // Write atomically, as vdirsyncer does: temp file, then rename.
    let tmp = list.join(".new.ics.tmp");
    std::fs::write(&tmp, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
    std::fs::rename(&tmp, list.join("new.ics")).unwrap();

    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
}

/// Reading the vdir (as every reload does) must not count as a change, or the TUI reloads
/// in a loop and resets the sidebar cursor.
#[test]
fn reading_files_does_not_signal() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("list");
    std::fs::create_dir(&list).unwrap();
    let file = list.join("a.ics");
    std::fs::write(&file, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
    let (_debouncer, rx) = watch(dir.path()).unwrap();
    while rx.recv_timeout(Duration::from_millis(800)).is_ok() {}

    twydo_core::vdir::load(dir.path()).unwrap();
    std::fs::read(&file).unwrap();

    assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
}
