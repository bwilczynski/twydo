//! Exercise item moves through real key handling, the daemon's write path, queue and reload.
mod common;

use chrono::{TimeZone, Utc};
use common::WriteDaemon;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{fs, path::Path};
use tdo::{
    app::{App, Outcome, Pane},
    write::Request,
};
use twydo_core::{
    compat::{Matrix, OperationEntry, Verdict},
    config::Config,
    edit::{Op, Placement},
    model::TaskKind,
    store::queue,
    views::Snapshot,
};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 17, 0, 0).unwrap()
}
fn matrix() -> Matrix {
    let mut m = Matrix::embedded().clone();
    for case in ["checklist", "project"] {
        for op in ["reorder", "insert"] {
            m.operation.retain(|r| !(r.case == case && r.op == op));
            m.operation.push(OperationEntry {
                case: case.into(),
                op: op.into(),
                iphone_baseline: Verdict::Unsupported,
                mac_to_iphone: Verdict::Unverified,
                iphone_to_mac: Verdict::Unverified,
                twydo_to_iphone: Verdict::Pass,
                iphone_to_twydo: Verdict::Pass,
                evidence: vec![],
                notes: String::new(),
            });
        }
    }
    m
}
struct Env {
    _tmp: tempfile::TempDir,
    cfg: Config,
    parent: String,
}
impl Env {
    fn new(kind: TaskKind) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let vdir = tmp.path().join("vdir");
        fs::create_dir_all(vdir.join("list1")).unwrap();
        let scene = if kind == TaskKind::Checklist {
            "issue35-checklist-order"
        } else {
            "issue35-project-order"
        };
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1")
            .join(scene)
            .join("000-created");
        for e in fs::read_dir(src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|e| e == "ics") {
                fs::copy(&p, vdir.join("list1").join(p.file_name().unwrap())).unwrap();
            }
        }
        let mut cfg = Config::with_vdir(vdir);
        cfg.timezone = Some("UTC".into());
        cfg.state_dir = Some(tmp.path().join("state"));
        let s = Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone());
        let parent = s
            .tasks
            .iter()
            .find(|t| t.parent.is_none())
            .unwrap()
            .uid
            .clone();
        Self {
            _tmp: tmp,
            cfg,
            parent,
        }
    }
    fn snap(&self) -> Snapshot {
        queue::view(
            twydo_core::vdir::load(&self.cfg.vdir).unwrap(),
            self.cfg.clone(),
        )
    }
    fn app(&self) -> App {
        let mut a = App::new(self.snap(), now().with_timezone(&chrono_tz::UTC));
        a.handle_key(key('2'));
        a.focus = Pane::Tasks;
        a.expanded.insert(self.parent.clone());
        a
    }
    fn writer(&self) -> WriteDaemon {
        WriteDaemon::start(self.cfg.clone(), matrix())
    }
}
fn select(a: &mut App, title: &str) {
    a.cursor = a
        .visible_rows()
        .iter()
        .position(|r| a.snapshot.tasks[r.task].summary == title)
        .unwrap();
}
fn titles(a: &App) -> Vec<String> {
    a.visible_rows()
        .iter()
        .filter(|r| r.depth > 0)
        .map(|r| a.snapshot.tasks[r.task].summary.clone())
        .collect()
}
fn apply(_env: &Env, a: &mut App, w: &WriteDaemon, out: Outcome) {
    let Outcome::Write(req) = out else {
        panic!("expected write: {out:?}")
    };
    let result = w.run(&req);
    a.reload(w.snapshot());
    a.write_finished(result);
}

#[test]
fn keys_reorder_and_preserve_selection_with_undo_redo() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        let env = Env::new(kind);
        let mut a = env.app();
        let w = env.writer();
        select(&mut a, "Mike");
        let uid = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
        let out = a.handle_key(key('K'));
        apply(&env, &mut a, &w, out);
        assert_eq!(titles(&a), ["Zulu", "Mike", "Alpha"]);
        assert_eq!(a.snapshot.tasks[a.selected().unwrap()].uid, uid);
        let out = a.handle_key(key('u'));
        apply(&env, &mut a, &w, out);
        assert_eq!(titles(&a), ["Zulu", "Alpha", "Mike"]);
        let out = a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        apply(&env, &mut a, &w, out);
        assert_eq!(titles(&a), ["Zulu", "Mike", "Alpha"]);
    }
}

#[test]
fn filtered_move_skips_hidden_siblings_and_contexts_do_not_write() {
    let env = Env::new(TaskKind::Checklist);
    let mut a = env.app();
    let w = env.writer();
    // Search notes shared by Zulu/Mike only, so Alpha is hidden.
    let mut s = env.snap();
    for t in &mut s.tasks {
        if ["Zulu", "Mike"].contains(&t.summary.as_str()) {
            t.description = Some("visible".into());
        }
    }
    a.reload(s);
    a.search = Some("visible".into());
    select(&mut a, "Zulu");
    let Outcome::Write(Request::Apply(Op::Reorder {
        uid,
        anchor,
        placement,
    })) = a.handle_key(key('J'))
    else {
        panic!("expected reorder")
    };
    assert_eq!(placement, Placement::After);
    assert_eq!(
        a.snapshot.tasks[a.snapshot.find(&anchor).unwrap()].summary,
        "Mike"
    );
    // Use disk snapshot: the search-only notes are an in-memory visibility fixture.
    let req = Request::Apply(Op::Reorder {
        uid,
        anchor,
        placement,
    });
    w.run(&req).unwrap();
    a.search = None;
    a.reload(w.snapshot());
    assert_eq!(titles(&a), ["Alpha", "Mike", "Zulu"]);
    for pane in [Pane::Sidebar, Pane::Details] {
        a.focus = pane;
        assert_eq!(a.handle_key(key('J')), Outcome::Continue);
    }
    a.focus = Pane::Tasks;
    select(&mut a, "Zulu");
    assert_eq!(a.handle_key(key('J')), Outcome::Continue);
    a.show_help = true;
    assert_eq!(a.handle_key(key('K')), Outcome::Continue);
}

#[test]
fn insertion_position_survives_row_and_card_and_chain_appends() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        for card in [false, true] {
            for (c, want) in [
                ('o', vec!["Zulu", "Alpha", "Inserted", "Mike"]),
                ('O', vec!["Zulu", "Inserted", "Alpha", "Mike"]),
            ] {
                let env = Env::new(kind);
                let mut a = env.app();
                let w = env.writer();
                select(&mut a, "Alpha");
                a.handle_key(key(c));
                for c in "Inserted".chars() {
                    a.handle_key(key(c));
                }
                if card {
                    a.handle_key(code(KeyCode::Tab));
                }
                let out = a.handle_key(if card {
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
                } else {
                    code(KeyCode::Enter)
                });
                apply(&env, &mut a, &w, out);
                assert_eq!(titles(&a), want);
                if !card {
                    for c in "Append".chars() {
                        a.handle_key(key(c));
                    }
                    let out = a.handle_key(code(KeyCode::Enter));
                    apply(&env, &mut a, &w, out);
                    assert_eq!(titles(&a).last().unwrap(), "Append");
                }
            }
        }
    }
}

#[test]
fn queued_move_displays_immediately_and_commits_after_sync() {
    let env = Env::new(TaskKind::Project);
    let mut a = env.app();
    let w = env.writer();
    let state = twydo_core::store::bind_vdir_state(&env.cfg.state_dir(), &env.cfg.vdir).unwrap();
    let lock = w
        .handle
        .hold_long(twydo_daemon::server::lane::LongKind::Test);
    select(&mut a, "Mike");
    let out = a.handle_key(key('K'));
    apply(&env, &mut a, &w, out);
    assert_eq!(titles(&a), ["Zulu", "Mike", "Alpha"]);
    assert!(a.message.as_deref().unwrap().contains("queued"));
    drop(lock);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !queue::Queue::existing(&state).is_none_or(|q| q.is_empty().unwrap()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the queue did not drain"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    a.reload(w.snapshot());
    assert_eq!(titles(&a), ["Zulu", "Mike", "Alpha"]);
}

#[test]
fn stale_insertion_anchor_keeps_typed_text_in_row_or_card() {
    for card in [false, true] {
        let env = Env::new(TaskKind::Checklist);
        let mut a = env.app();
        let w = env.writer();
        select(&mut a, "Alpha");
        let uid = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
        a.handle_key(key('O'));
        for c in "Keep typed".chars() {
            a.handle_key(key(c));
        }
        if card {
            a.handle_key(code(KeyCode::Tab));
        }
        fs::remove_file(env.cfg.vdir.join(format!("list1/{uid}.ics"))).unwrap();
        a.reload(w.snapshot());
        let out = a.handle_key(if card {
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        } else {
            code(KeyCode::Enter)
        });
        apply(&env, &mut a, &w, out);
        if card {
            let f = a.form.as_ref().unwrap();
            assert_eq!(f.title, "Keep typed");
            assert!(f.error.is_some());
        } else {
            let r = a.new_row.as_ref().unwrap();
            assert_eq!(r.title, "Keep typed");
            assert!(r.error.is_some());
        }
        assert!(!env.snap().tasks.iter().any(|t| t.summary == "Keep typed"));
    }
}

#[test]
fn boundaries_overlays_detached_rows_and_flat_projects_do_not_write() {
    let env = Env::new(TaskKind::Project);
    let mut a = env.app();
    select(&mut a, "Zulu");
    assert_eq!(a.handle_key(key('K')), Outcome::Continue);
    for overlay in ["help", "errors", "conflicts"] {
        a.show_help = overlay == "help";
        a.show_errors = overlay == "errors";
        a.show_conflicts = overlay == "conflicts";
        assert_eq!(a.handle_key(key('J')), Outcome::Continue);
    }
    a.show_help = false;
    a.show_errors = false;
    a.show_conflicts = false;
    let mut s = env.snap();
    for t in &mut s.tasks {
        if t.parent.is_some() {
            t.due = Some(twydo_core::ical::value::IcalDateTime::Date(
                now().date_naive(),
            ));
        }
    }
    a.reload(s);
    a.handle_key(key('1'));
    select(&mut a, "Zulu");
    assert_eq!(a.handle_key(key('J')), Outcome::Continue);
    a.handle_key(key('2'));
    let mut s = env.snap();
    let i = s.tasks.iter().position(|t| t.summary == "Zulu").unwrap();
    s.tasks[i].status = twydo_core::model::Status::Completed;
    s.tasks[i].completed = Some(twydo_core::ical::value::IcalDateTime::Utc(now()));
    a.reload(s);
    select(&mut a, "Zulu");
    assert_eq!(a.handle_key(key('J')), Outcome::Continue);
}

#[test]
fn tasks_and_details_render_the_same_reordered_sequence_and_counts() {
    use ratatui::{Terminal, backend::TestBackend};
    let env = Env::new(TaskKind::Project);
    let mut a = env.app();
    let w = env.writer();
    let counts = twydo_core::views::counts(&a.snapshot, &twydo_core::views::View::All, a.now);
    select(&mut a, "Mike");
    let out = a.handle_key(key('K'));
    apply(&env, &mut a, &w, out);
    assert_eq!(
        twydo_core::views::counts(&a.snapshot, &twydo_core::views::View::All, a.now),
        counts
    );
    a.cursor = 0;
    a.resize(180, 40);
    let mut terminal = Terminal::new(TestBackend::new(180, 40)).unwrap();
    terminal.draw(|f| tdo::ui::draw(f, &a)).unwrap();
    let buf = terminal.backend().buffer();
    let mut tasks = Vec::new();
    let mut details = Vec::new();
    for y in 0..40 {
        for (range, lines) in [(24..110, &mut tasks), (115..180, &mut details)] {
            let line: String = range.map(|x| buf[(x, y)].symbol()).collect();
            for name in ["Zulu", "Mike", "Alpha"] {
                if line.contains(name) {
                    lines.push(name);
                }
            }
        }
    }
    assert_eq!(tasks, ["Zulu", "Mike", "Alpha"]);
    assert_eq!(details, ["Zulu", "Mike", "Alpha"]);
}

#[test]
fn normal_verified_tui_can_reorder_items_of_a_recurring_checklist() {
    let mut env = Env::new(TaskKind::Checklist);
    for entry in fs::read_dir(env.cfg.vdir.join("list1")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let parent = "61ed74be3ba34212ab7476b698de1263";
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan8-tui-a/010-first-phone-actions");
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if !path.extension().is_some_and(|e| e == "ics") {
            continue;
        }
        let cal = twydo_core::ical::parse(&fs::read(&path).unwrap()).unwrap();
        let todo = cal.root.children_named("VTODO").next().unwrap();
        if todo.prop("UID").unwrap().value == parent
            || todo.prop("RELATED-TO").is_some_and(|p| p.value == parent)
        {
            fs::copy(
                &path,
                env.cfg.vdir.join("list1").join(path.file_name().unwrap()),
            )
            .unwrap();
        }
    }
    env.parent = parent.into();
    let mut a = env.app();
    assert!(
        a.snapshot.tasks[a.snapshot.find(parent).unwrap()]
            .rrule
            .is_some()
    );
    let nested: Vec<_> = a
        .visible_rows()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.depth > 0)
        .map(|(n, r)| (n, a.snapshot.tasks[r.task].uid.clone()))
        .collect();
    assert!(nested.len() >= 2);
    a.cursor = nested.last().unwrap().0;
    let selected = nested.last().unwrap().1.clone();
    let mut want: Vec<_> = nested.iter().map(|(_, uid)| uid.clone()).collect();
    let last = want.len() - 1;
    want.swap(last, last - 1);
    let writer = WriteDaemon::start(env.cfg.clone(), Matrix::embedded().clone());
    let out = a.handle_key(key('K'));
    apply(&env, &mut a, &writer, out);
    let actual: Vec<_> = a
        .visible_rows()
        .iter()
        .filter(|r| r.depth > 0)
        .map(|r| a.snapshot.tasks[r.task].uid.clone())
        .collect();
    assert_eq!(actual, want);
    assert_eq!(a.snapshot.tasks[a.selected().unwrap()].uid, selected);
}
