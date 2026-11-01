//! Sync end to end: real vdirsyncer between two filesystem storages, `tdo` as its conflict
//! hook. Content is synthetic. Every test skips when vdirsyncer is not installed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use twydo_core::conflicts::{ConflictKind, ConflictState, ConflictStore, Side};
use twydo_core::sync::hook::{HOOK_LOG_ENV, read_hook_log};

/// Collection used by [`Pair`].
const COLL: &str = "c1";

/// A synthetic task resource.
fn task(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//twydo test//EN\r\nBEGIN:VTODO\r\n\
         UID:{uid}\r\nSUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

fn vdirsyncer_installed() -> bool {
    Command::new("vdirsyncer")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Returns early from the test when vdirsyncer is missing.
macro_rules! require_vdirsyncer {
    () => {
        if !vdirsyncer_installed() {
            eprintln!("skipping: vdirsyncer not installed");
            return;
        }
    };
}

/// A vdirsyncer pair `twydo` between `server/` (storage `twydo_remote`, side a) and `vdir/`
/// (storage `twydo_local`, side b), both holding collection `c1`, with `tdo conflicts hook`
/// as the conflict command, and a tdo config (test write mode) whose `[sync]` points at that
/// vdirsyncer config. `new` syncs three tasks `u1`..`u3` once.
struct Pair {
    dir: tempfile::TempDir,
}

#[allow(dead_code)]
impl Pair {
    fn new() -> Pair {
        Pair::with_sync("")
    }

    /// Like [`Pair::new`], with `extra` lines appended to the `[sync]` table.
    fn with_sync(extra: &str) -> Pair {
        let dir = tempfile::tempdir().unwrap();
        let p = Pair { dir };
        fs::create_dir_all(p.server().join(COLL)).unwrap();
        fs::create_dir_all(p.vdir().join(COLL)).unwrap();
        fs::create_dir_all(p.root().join("vdirsyncer")).unwrap();
        let hook = env!("CARGO_BIN_EXE_tdo");
        fs::write(
            p.vdirsyncer_config(),
            format!(
                "[general]\nstatus_path = \"{status}/\"\n\n\
                 [pair twydo]\na = \"twydo_remote\"\nb = \"twydo_local\"\n\
                 collections = [\"from a\", \"from b\"]\n\
                 conflict_resolution = [\"command\", \"{hook}\", \"conflicts\", \"hook\"]\n\n\
                 [storage twydo_remote]\ntype = \"filesystem\"\npath = \"{server}/\"\nfileext = \".ics\"\n\n\
                 [storage twydo_local]\ntype = \"filesystem\"\npath = \"{vdir}/\"\nfileext = \".ics\"\n",
                status = p.root().join("vdirsyncer/status").display(),
                server = p.server().display(),
                vdir = p.vdir().display(),
            ),
        )
        .unwrap();
        fs::create_dir_all(p.root().join("config/twydo")).unwrap();
        fs::write(
            p.root().join("config/twydo/config.toml"),
            format!(
                "vdir = \"{}\"\ntimezone = \"UTC\"\n[writes]\nmode = \"test\"\n\
                 [sync]\nvdirsyncer_config = \"{}\"\nafter_edit = false\n{extra}",
                p.vdir().display(),
                p.vdirsyncer_config().display()
            ),
        )
        .unwrap();
        let mut matrix = String::from("matrix_version = 1\n");
        for op in ["create", "edit", "delete"] {
            matrix.push_str(&format!(
                "[[operation]]\ncase = \"one-time\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"pass\"\niphone_to_mac = \"pass\"\n\
                 twydo_to_iphone = \"pass\"\niphone_to_twydo = \"pass\"\n"
            ));
        }
        fs::write(p.root().join("matrix.toml"), matrix).unwrap();
        for (uid, s) in [("u1", "one"), ("u2", "two"), ("u3", "three")] {
            p.write_vdir(uid, &task(uid, s));
        }
        let (code, err) = p.vdirsyncer(&["discover", "twydo"]);
        assert_eq!(code, 0, "discover: {err}");
        let (code, err) = p.sync();
        assert_eq!(code, 0, "initial sync: {err}");
        assert_eq!(p.read_server("u3"), Some(task("u3", "three")));
        p
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn server(&self) -> PathBuf {
        self.root().join("server")
    }

    fn vdir(&self) -> PathBuf {
        self.root().join("vdir")
    }

    fn vdirsyncer_config(&self) -> PathBuf {
        self.root().join("vdirsyncer/config")
    }

    /// `XDG_STATE_HOME` of `tdo`.
    fn state(&self) -> PathBuf {
        self.root().join("state")
    }

    /// The per-vdir state dir `tdo` uses (journal, lock, conflicts).
    fn vdir_state(&self) -> PathBuf {
        twydo_core::store::vdir_state_dir(&self.state().join("twydo"), &self.vdir()).unwrap()
    }

    fn conflicts(&self) -> ConflictStore {
        ConflictStore::open(&self.vdir_state()).unwrap()
    }

    fn write_server(&self, uid: &str, content: &str) {
        fs::write(self.server().join(COLL).join(format!("{uid}.ics")), content).unwrap();
    }

    fn write_vdir(&self, uid: &str, content: &str) {
        fs::write(self.vdir().join(COLL).join(format!("{uid}.ics")), content).unwrap();
    }

    fn read_server(&self, uid: &str) -> Option<String> {
        fs::read_to_string(self.server().join(COLL).join(format!("{uid}.ics"))).ok()
    }

    fn read_vdir(&self, uid: &str) -> Option<String> {
        fs::read_to_string(self.vdir().join(COLL).join(format!("{uid}.ics"))).ok()
    }

    fn env(&self, cmd: &mut Command) {
        cmd.env("VDIRSYNCER_CONFIG", self.vdirsyncer_config())
            .env("XDG_CONFIG_HOME", self.root().join("config"))
            .env("XDG_STATE_HOME", self.state())
            .env_remove(HOOK_LOG_ENV)
            .stdin(Stdio::null());
    }

    /// Runs vdirsyncer with extra environment; returns (exit code, stderr).
    fn vdirsyncer_env(&self, args: &[&str], extra: &[(&str, &Path)]) -> (i32, String) {
        let mut cmd = Command::new("vdirsyncer");
        cmd.args(args);
        self.env(&mut cmd);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let o = cmd.output().expect("run vdirsyncer");
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    fn vdirsyncer(&self, args: &[&str]) -> (i32, String) {
        self.vdirsyncer_env(args, &[])
    }

    fn sync(&self) -> (i32, String) {
        self.vdirsyncer(&["sync", "twydo"])
    }

    fn tdo(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tdo"));
        cmd.args(args);
        self.env(&mut cmd);
        cmd.output().expect("run tdo")
    }

    /// `tdo op --matrix <test matrix> args…`; asserts success and returns the op id.
    fn op(&self, args: &[&str]) -> String {
        let m = self.root().join("matrix.toml");
        let mut a = vec!["op", "--matrix", m.to_str().unwrap()];
        a.extend_from_slice(args);
        let o = self.tdo(&a);
        assert!(o.status.success(), "tdo {a:?}: {o:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// `tdo sync`; returns (exit code, stderr).
    fn tdo_sync(&self) -> (i32, String) {
        let o = self.tdo(&["sync"]);
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    /// The `tdo op journal` line of entry `id`, if any.
    fn journal_line(&self, id: &str) -> Option<String> {
        let o = self.tdo(&["op", "journal"]);
        assert!(o.status.success(), "{o:?}");
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .find(|l| l.starts_with(id))
            .map(str::to_string)
    }

    fn synced(&self, id: &str) -> bool {
        let line = self.journal_line(id).expect("journal entry");
        assert!(
            line.ends_with("pending") || line.contains("  synced "),
            "{line}"
        );
        line.contains("  synced ")
    }

    fn conflict_dir(&self, id: &str) -> PathBuf {
        self.vdir_state().join("conflicts").join(id)
    }
}

/// Edits `u1` on both sides and `u2` on the server only.
fn make_conflict(p: &Pair) -> (String, String) {
    let local = task("u1", "one-local");
    let remote = task("u1", "one-remote");
    p.write_vdir("u1", &local);
    p.write_server("u1", &remote);
    p.write_server("u2", &task("u2", "two-remote"));
    (local, remote)
}

#[test]
fn hook_records_both_versions() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (local, remote) = make_conflict(&p);
    let log = p.root().join("hook.log");
    let (code, err) = p.vdirsyncer_env(&["sync", "twydo"], &[(HOOK_LOG_ENV, &log)]);
    assert_eq!(code, 1, "{err}");

    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    let c = &open[0];
    assert_eq!(c.state, ConflictState::Open);
    assert_eq!((c.uid.as_str(), c.collection.as_str()), ("u1", COLL));
    assert_eq!(c.path, format!("{COLL}/u1.ics"));
    assert!(c.held);
    assert_eq!(c.vdir_holds, Side::Local);
    let dir = p.vdir_state().join("conflicts").join(&c.id);
    assert_eq!(fs::read_to_string(dir.join("local.ics")).unwrap(), local);
    assert_eq!(fs::read_to_string(dir.join("remote.ics")).unwrap(), remote);

    // Both sides keep their own version; the other item synced.
    assert_eq!(p.read_vdir("u1"), Some(local));
    assert_eq!(p.read_server("u1"), Some(remote));
    assert_eq!(p.read_vdir("u2"), Some(task("u2", "two-remote")));

    let records = read_hook_log(&log).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].uid, "u1");
    assert_eq!(records[0].collection, COLL);
    assert_eq!(records[0].conflict, c.id);
    assert!(!records[0].resolved);
}

#[test]
fn hook_does_not_duplicate_on_rerun() {
    require_vdirsyncer!();
    let p = Pair::new();
    make_conflict(&p);
    assert_eq!(p.sync().0, 1);
    let first = p.conflicts().unresolved().unwrap();
    let log = p.root().join("hook.log");
    let (code, err) = p.vdirsyncer_env(&["sync", "twydo"], &[(HOOK_LOG_ENV, &log)]);
    assert_eq!(code, 1, "{err}");
    let all = p.conflicts().list().unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].id, first[0].id);
    assert_eq!(all[0].state, ConflictState::Open);
    let records = read_hook_log(&log).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].conflict, first[0].id);
    assert!(!records[0].resolved);
}

fn chosen_is_applied(side: Side) {
    let p = Pair::new();
    let (local, remote) = make_conflict(&p);
    assert_eq!(p.sync().0, 1);
    let id = p.conflicts().unresolved().unwrap()[0].id.clone();
    p.conflicts().choose(&id, side).unwrap();

    let log = p.root().join("hook.log");
    let (code, err) = p.vdirsyncer_env(&["sync", "twydo"], &[(HOOK_LOG_ENV, &log)]);
    assert_eq!(code, 0, "{err}");
    let kept = match side {
        Side::Local => local,
        Side::Remote => remote,
    };
    // vdirsyncer 0.21.0 reads the hook's files back in Python text mode, so the item it
    // writes has its CRLF line endings turned into LF. The side that already held the chosen
    // version is left alone (vdirsyncer's item hash ignores line endings); only the other
    // side is rewritten.
    let applied = kept.replace("\r\n", "\n");
    let (vdir, server) = match side {
        Side::Local => (kept, applied),
        Side::Remote => (applied, kept),
    };
    assert_eq!(p.read_vdir("u1"), Some(vdir));
    assert_eq!(p.read_server("u1"), Some(server));
    // The hook applies the choice; marking it resolved is the runner's job.
    let c = p.conflicts().get(&id).unwrap().unwrap();
    assert_eq!(c.state, ConflictState::Chosen);
    let records = read_hook_log(&log).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].conflict, id);
    assert!(records[0].resolved);
}

#[test]
fn chosen_remote_is_applied_by_hook() {
    require_vdirsyncer!();
    chosen_is_applied(Side::Remote);
}

#[test]
fn chosen_local_is_applied_by_hook() {
    require_vdirsyncer!();
    chosen_is_applied(Side::Local);
}

#[test]
fn remote_changed_again_supersedes() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (local, _) = make_conflict(&p);
    assert_eq!(p.sync().0, 1);
    let old = p.conflicts().unresolved().unwrap()[0].id.clone();
    p.conflicts().choose(&old, Side::Remote).unwrap();
    let newer = task("u1", "one-remote-again");
    p.write_server("u1", &newer);

    let (code, err) = p.sync();
    assert_eq!(code, 1, "{err}");
    let c = p.conflicts().get(&old).unwrap().unwrap();
    assert_eq!(c.state, ConflictState::Superseded);
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_ne!(open[0].id, old);
    assert_eq!(open[0].state, ConflictState::Open);
    assert_eq!(open[0].local.as_deref(), Some(local.as_str()));
    assert_eq!(open[0].remote.as_deref(), Some(newer.as_str()));
    assert_eq!(p.read_vdir("u1"), Some(local));
    assert_eq!(p.read_server("u1"), Some(newer));
}

#[test]
fn unknown_side_exits_2_and_changes_nothing() {
    require_vdirsyncer!();
    let p = Pair::new();
    let x = p.root().join("x");
    let y = p.root().join("y");
    fs::write(&x, task("u1", "x")).unwrap();
    fs::write(&y, task("u1", "y")).unwrap();
    let o = p.tdo(&[
        "conflicts",
        "hook",
        x.to_str().unwrap(),
        y.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("local_storage"),
        "{o:?}"
    );
    assert_eq!(fs::read_to_string(&x).unwrap(), task("u1", "x"));
    assert_eq!(fs::read_to_string(&y).unwrap(), task("u1", "y"));
    assert!(p.conflicts().list().unwrap().is_empty());
}

#[test]
fn edit_is_confirmed_after_sync() {
    require_vdirsyncer!();
    let p = Pair::new();
    let id = p.op(&["edit", "u1", "--title", "one-edited"]);
    assert!(!p.synced(&id));
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 0, "{err}");
    assert!(p.synced(&id));
    let edited = p.read_vdir("u1").unwrap();
    assert!(edited.contains("SUMMARY:one-edited"), "{edited}");
    assert_eq!(p.read_server("u1"), Some(edited));
    assert!(p.conflicts().list().unwrap().is_empty());
}

#[test]
fn local_delete_vs_remote_edit_becomes_conflict() {
    require_vdirsyncer!();
    let p = Pair::new();
    let id = p.op(&["delete", "u2"]);
    let edit = task("u2", "two-remote");
    p.write_server("u2", &edit);
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 1, "{err}");
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    let c = &open[0];
    assert_eq!(c.kind, ConflictKind::LocalDeleteRemoteEdit);
    assert!(!c.held);
    assert_eq!(c.vdir_holds, Side::Remote);
    assert_eq!((c.uid.as_str(), c.path.as_str()), ("u2", "c1/u2.ics"));
    let dir = p.conflict_dir(&c.id);
    assert_eq!(fs::read_to_string(dir.join("remote.ics")).unwrap(), edit);
    assert!(!dir.join("local.ics").exists());
    assert!(p.synced(&id));
}

#[test]
fn remote_delete_vs_local_edit_becomes_conflict() {
    require_vdirsyncer!();
    let p = Pair::new();
    let id = p.op(&["edit", "u3", "--title", "three-local"]);
    let edit = p.read_vdir("u3").unwrap();
    fs::remove_file(p.server().join(COLL).join("u3.ics")).unwrap();
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 1, "{err}");
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    let c = &open[0];
    assert_eq!(c.kind, ConflictKind::RemoteDeleteLocalEdit);
    assert!(!c.held);
    assert_eq!(c.vdir_holds, Side::Local);
    assert_eq!((c.uid.as_str(), c.path.as_str()), ("u3", "c1/u3.ics"));
    let dir = p.conflict_dir(&c.id);
    assert_eq!(fs::read_to_string(dir.join("local.ics")).unwrap(), edit);
    assert!(!dir.join("remote.ics").exists());
    assert!(p.synced(&id));
}

#[test]
fn created_task_upload_is_not_a_conflict() {
    require_vdirsyncer!();
    let p = Pair::new();
    let id = p.op(&["add", "--list", COLL, "--title", "four"]);
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 0, "{err}");
    assert!(p.conflicts().list().unwrap().is_empty());
    assert!(p.synced(&id));
}

/// Twydo edits `u1` and the server edits it too; `tdo sync` holds the conflict.
fn held_conflict(p: &Pair) -> (String, String) {
    let op = p.op(&["edit", "u1", "--title", "one-local"]);
    p.write_server("u1", &task("u1", "one-remote"));
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 1, "{err}");
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0].held);
    assert!(!p.synced(&op), "a held conflict keeps the entry pending");
    (op, open[0].id.clone())
}

#[test]
fn held_conflict_keeps_entry_pending_until_resolved() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (op, id) = held_conflict(&p);
    // Still held on a rerun.
    assert_eq!(p.tdo_sync().0, 1);
    assert!(!p.synced(&op));
    p.conflicts().choose(&id, Side::Remote).unwrap();
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 0, "{err}");
    assert!(p.synced(&op));
    let c = p.conflicts().get(&id).unwrap().unwrap();
    assert_eq!(c.state, ConflictState::Resolved);
    assert_eq!(
        p.read_vdir("u1"),
        Some(task("u1", "one-remote").replace("\r\n", "\n"))
    );
}

#[test]
fn choice_applied_by_manual_vdirsyncer_resolves() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (op, id) = held_conflict(&p);
    p.conflicts().choose(&id, Side::Remote).unwrap();
    // vdirsyncer by hand: the hook applies the choice but writes no hook log.
    let (code, err) = p.sync();
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        p.conflicts().get(&id).unwrap().unwrap().state,
        ConflictState::Chosen
    );
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        p.conflicts().get(&id).unwrap().unwrap().state,
        ConflictState::Resolved
    );
    assert!(p.synced(&op));
}

#[test]
fn agreeing_sides_resolve_open_conflict() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (op, id) = held_conflict(&p);
    p.write_server("u1", &p.read_vdir("u1").unwrap());
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 0, "{err}");
    let c = p.conflicts().get(&id).unwrap().unwrap();
    assert_eq!(c.state, ConflictState::Resolved);
    assert_eq!(c.choice, None);
    assert!(p.synced(&op));
}

#[test]
fn prune_after_retention() {
    require_vdirsyncer!();
    let p = Pair::with_sync("retention_days = 0\n");
    let id = p.op(&["edit", "u1", "--title", "one-edited"]);
    assert_eq!(p.tdo_sync().0, 0);
    assert!(p.synced(&id));
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert_eq!(p.tdo_sync().0, 0);
    assert_eq!(p.journal_line(&id), None);
}

#[test]
fn server_delete_of_held_item_is_not_lost() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (op, id) = held_conflict(&p);
    let local = p.read_vdir("u1").unwrap();
    // The phone deletes the conflicted task; vdirsyncer re-uploads the local edit (F6).
    fs::remove_file(p.server().join(COLL).join("u1.ics")).unwrap();
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        p.conflicts().get(&id).unwrap().unwrap().state,
        ConflictState::Superseded
    );
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].kind, ConflictKind::RemoteDeleteLocalEdit);
    assert_eq!(open[0].local.as_deref(), Some(local.as_str()));
    assert!(p.synced(&op));
}

#[test]
fn twydo_delete_of_held_item_is_not_lost() {
    require_vdirsyncer!();
    let p = Pair::new();
    let (edit, id) = held_conflict(&p);
    let del = p.op(&["delete", "u1"]);
    // vdirsyncer restores the server's edit locally (F6).
    let (code, err) = p.tdo_sync();
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        p.conflicts().get(&id).unwrap().unwrap().state,
        ConflictState::Superseded
    );
    let open = p.conflicts().unresolved().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].kind, ConflictKind::LocalDeleteRemoteEdit);
    assert_eq!(
        open[0].remote.as_deref(),
        Some(task("u1", "one-remote").as_str())
    );
    assert!(p.synced(&edit));
    assert!(p.synced(&del));
}

/// `tdo sync` with extra args; returns (exit code, stdout, stderr).
fn tdo_sync_args(p: &Pair, extra: &[&str]) -> (i32, String, String) {
    let mut a = vec!["sync"];
    a.extend_from_slice(extra);
    let o = p.tdo(&a);
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn coll_state(p: &Pair) -> String {
    let o = p.tdo(&["status"]);
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn twydo_emptying_a_list_is_forced() {
    require_vdirsyncer!();
    let p = Pair::new();
    for u in ["u1", "u2", "u3"] {
        p.op(&["delete", u]);
    }
    let (code, out, err) = tdo_sync_args(&p, &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("applied the emptied list \"c1\" on the server"),
        "{out}"
    );
    for u in ["u1", "u2", "u3"] {
        assert_eq!(p.read_server(u), None);
    }
    assert!(coll_state(&p).contains("c1: ok"), "{}", coll_state(&p));
}

#[test]
fn remote_emptied_is_never_forced() {
    require_vdirsyncer!();
    let p = Pair::new();
    for u in ["u1", "u2", "u3"] {
        fs::remove_file(p.server().join(COLL).join(format!("{u}.ics"))).unwrap();
    }
    let (code, out, err) = tdo_sync_args(&p, &[]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        err.contains(
            "list \"c1\" was emptied on the server; to apply that here run: tdo sync --force-delete \"c1\""
        ),
        "{err}"
    );
    assert!(p.read_vdir("u1").is_some());
    assert!(coll_state(&p).contains("c1: emptied on the server"));
    let (code, out, err) = tdo_sync_args(&p, &["--force-delete", "c1"]);
    assert_eq!(code, 0, "{out}{err}");
    for u in ["u1", "u2", "u3"] {
        assert_eq!(p.read_vdir(u), None);
    }
    assert!(!out.contains("applied the emptied list"), "{out}");
}

#[test]
fn local_emptied_without_journal_is_not_forced() {
    require_vdirsyncer!();
    let p = Pair::new();
    for u in ["u1", "u2", "u3"] {
        fs::remove_file(p.vdir().join(COLL).join(format!("{u}.ics"))).unwrap();
    }
    let (code, out, err) = tdo_sync_args(&p, &[]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        err.contains("was emptied here; to apply that on the server run: tdo sync --force-delete"),
        "{err}"
    );
    assert!(coll_state(&p).contains("c1: emptied here"));
    assert_eq!(p.read_server("u1"), Some(task("u1", "one")));
}
