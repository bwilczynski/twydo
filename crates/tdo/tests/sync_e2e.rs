//! Sync end to end: real vdirsyncer between two filesystem storages, `tdo` as its conflict
//! hook. Content is synthetic. Every test skips when vdirsyncer is not installed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use twydo_core::conflicts::{ConflictState, ConflictStore, Side};
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
/// as the conflict command, and a tdo config whose `[sync]` points at that vdirsyncer config.
/// `new` syncs three tasks `u1`..`u3` once.
struct Pair {
    dir: tempfile::TempDir,
}

#[allow(dead_code)]
impl Pair {
    fn new() -> Pair {
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
                "vdir = \"{}\"\ntimezone = \"UTC\"\n[sync]\nvdirsyncer_config = \"{}\"\n",
                p.vdir().display(),
                p.vdirsyncer_config().display()
            ),
        )
        .unwrap();
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
