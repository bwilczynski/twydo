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
