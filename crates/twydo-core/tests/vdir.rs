use std::fs;
use std::path::Path;
use std::time::SystemTime;

use twydo_core::vdir::{load, parse_color};

fn sample() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/sample-vdir"
    ))
}

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

const GOOD: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:g1\r\nSUMMARY:Good\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

#[test]
fn loads_sample_vdir() {
    let r = load(sample()).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let got: Vec<_> = r
        .collections
        .iter()
        .map(|c| (c.id.as_str(), c.display_name.as_deref(), c.color))
        .collect();
    assert_eq!(
        got,
        vec![
            ("bills", Some("Bills"), Some((0xEF, 0x44, 0x44))),
            ("inbox", Some("Inbox"), Some((0x3B, 0x82, 0xF6))),
            ("personal", Some("Personal"), Some((0x22, 0xC5, 0x5E))),
        ]
    );
    assert_eq!(r.tasks.len(), 16);
    let n = |c: &str| r.tasks.iter().filter(|t| t.collection == c).count();
    assert_eq!((n("inbox"), n("bills"), n("personal")), (5, 6, 5));
    assert!(r.tasks.iter().all(|t| t.warnings.is_empty()));
}

#[test]
fn load_skips_bad_files_and_reports_them() {
    let d = tempfile::tempdir().unwrap();
    let c = d.path().join("work");
    write(&c.join("good.ics"), GOOD);
    write(
        &c.join("trunc.ics"),
        "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\n",
    );
    write(
        &c.join("event.ics"),
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    write(&c.join("good.ics.tmp"), "junk");
    write(&c.join(".hidden.ics"), GOOD);
    write(&d.path().join("stray.ics"), GOOD);
    let r = load(d.path()).unwrap();
    assert_eq!(r.tasks.len(), 1);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].0, c.join("trunc.ics"));
    assert!(!r.errors[0].1.is_empty());
    assert_eq!(r.collections[0].id, "work");
    assert_eq!(r.collections[0].display_name, None);
}

#[test]
fn bad_color_falls_back() {
    let d = tempfile::tempdir().unwrap();
    let c = d.path().join("work");
    write(&c.join("good.ics"), GOOD);
    write(&c.join("color"), "blue\n");
    write(&c.join("displayname"), "  Work  \n");
    let r = load(d.path()).unwrap();
    assert_eq!(r.collections[0].color, None);
    assert_eq!(r.collections[0].display_name.as_deref(), Some("Work"));
    assert_eq!(r.tasks.len(), 1);
}

#[test]
fn parse_color_forms() {
    assert_eq!(parse_color("#0a0B0c"), Some((10, 11, 12)));
    assert_eq!(parse_color(" #0A0B0CFF "), Some((10, 11, 12)));
    assert_eq!(parse_color("0A0B0C"), None);
    assert_eq!(parse_color("#12345"), None);
    assert_eq!(parse_color("#GGGGGG"), None);
    assert_eq!(parse_color("#é0000"), None);
}

#[test]
fn duplicate_uids_are_flagged_on_both() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("a/one.ics"), GOOD);
    write(&d.path().join("b/two.ics"), GOOD);
    let r = load(d.path()).unwrap();
    assert_eq!(r.tasks.len(), 2);
    assert!(r.tasks.iter().all(|t| t.warnings == ["duplicate UID"]));
}

#[test]
fn unreadable_root_is_error() {
    assert!(load(Path::new("/nonexistent/twydo-root")).is_err());
}

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, SystemTime, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let m = fs::metadata(&path).unwrap().modified().unwrap();
                out.push((path.clone(), m, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn loading_does_not_touch_files() {
    let before = snapshot(sample());
    load(sample()).unwrap();
    assert_eq!(before, snapshot(sample()));
}

#[cfg(unix)]
#[test]
fn load_follows_symlinks() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let real = d.path().join("elsewhere");
    write(&real.join("a.ics"), GOOD);
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    symlink(&real, root.join("linked")).unwrap();
    write(&d.path().join("target.ics"), &GOOD.replace("g1", "g2"));
    write(&root.join("plain/x.ics"), &GOOD.replace("g1", "g3"));
    symlink(d.path().join("target.ics"), root.join("plain/link.ics")).unwrap();
    symlink(
        d.path().join("missing.ics"),
        root.join("plain/dangling.ics"),
    )
    .unwrap();
    let r = load(&root).unwrap();
    assert_eq!(r.collections.len(), 2);
    assert_eq!(r.tasks.len(), 3);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].0, root.join("plain/dangling.ics"));
}

#[test]
fn overlay_replaces_creates_and_removes() {
    use twydo_core::store::journal::Write;
    let task = |uid: &str, title: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{title}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(&root.join("l/a.ics"), &task("a", "A"));
    write(&root.join("l/b.ics"), &task("b", "B"));
    write(&root.join("l/c.ics"), &task("c", "C"));
    let mut report = load(root).unwrap();
    let writes = [
        Write {
            path: "l/a.ics".into(),
            before: Some(task("a", "A")),
            after: Some(task("a", "A2")),
        },
        Write {
            path: "l/b.ics".into(),
            before: Some(task("b", "B")),
            after: None,
        },
        Write {
            path: "l/d.ics".into(),
            before: None,
            after: Some(task("d", "D")),
        },
    ];
    report.overlay_writes(root, &writes);

    write(&root.join("l/a.ics"), &task("a", "A2"));
    fs::remove_file(root.join("l/b.ics")).unwrap();
    write(&root.join("l/d.ics"), &task("d", "D"));
    let fresh = load(root).unwrap();
    assert_eq!(report.tasks, fresh.tasks);
    assert_eq!(report.sources, fresh.sources);
    assert_eq!(report.errors, fresh.errors);
}
