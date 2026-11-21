//! The sync manifest (`<state>/sync/manifest.json`): per collection, the vdir files
//! (`<coll>/<file>.ics`) as of the last run in which that collection counted. It is the
//! evidence the runner needs before it forces an emptied list on its own: only when the
//! journal removed every one of those files did Twydo empty the list.
//!
//! A list emptied on the server is never synced cleanly again until it is applied, so its
//! record could never be rebuilt once lost: the manifest marks it `held` and keeps it
//! through failed and killed runs until a run syncs the list again.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::status::CollState;
use crate::store::fsio::write_atomic;
use crate::store::journal::Entry;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    /// Collection id → its files, sorted.
    pub lists: BTreeMap<String, Vec<String>>,
    /// Lists emptied on the server whose record is kept until a run syncs them again.
    pub held: BTreeSet<String>,
}

/// The file format: `{"lists": {...}, "held": [...]}`, or the plain map of `lists` that
/// earlier versions wrote (nothing held).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum OnDisk {
    Current {
        lists: BTreeMap<String, Vec<String>>,
        #[serde(default)]
        held: BTreeSet<String>,
    },
    Plain(BTreeMap<String, Vec<String>>),
}

impl Manifest {
    /// `<state>/sync/manifest.json`.
    pub fn path(state: &Path) -> PathBuf {
        state.join("sync").join("manifest.json")
    }

    /// Reads `<state>/sync/manifest.json`; a missing file is an empty manifest.
    pub fn load(state: &Path) -> io::Result<Manifest> {
        let path = Manifest::path(state);
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(OnDisk::Current { lists, held }) => Ok(Manifest { lists, held }),
                Ok(OnDisk::Plain(lists)) => Ok(Manifest {
                    lists,
                    held: BTreeSet::new(),
                }),
                Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes `<state>/sync/manifest.json` atomically as pretty JSON.
    pub fn save(&self, state: &Path) -> io::Result<()> {
        fs::create_dir_all(state.join("sync"))?;
        let on_disk = OnDisk::Current {
            lists: self.lists.clone(),
            held: self.held.clone(),
        };
        let mut json = serde_json::to_string_pretty(&on_disk).map_err(io::Error::other)?;
        json.push('\n');
        write_atomic(&Manifest::path(state), json.as_bytes())
    }

    /// Replaces the entry of each of `colls` with a fresh listing of its directory in
    /// `vdir` (a missing directory lists nothing). Names that are not a plain directory
    /// name are skipped.
    pub fn refresh<'a>(
        &mut self,
        vdir: &Path,
        colls: impl IntoIterator<Item = &'a str>,
    ) -> io::Result<()> {
        for coll in colls {
            if !plain_name(coll) {
                continue;
            }
            let mut files = Vec::new();
            match fs::read_dir(vdir.join(coll)) {
                Ok(dir) => {
                    for d in dir {
                        let d = d?;
                        let Some(name) = d.file_name().to_str().map(str::to_string) else {
                            continue;
                        };
                        if name.starts_with('.') || !name.ends_with(".ics") {
                            continue;
                        }
                        if d.file_type()?.is_file() {
                            files.push(format!("{coll}/{name}"));
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            files.sort();
            self.lists.insert(coll.to_string(), files);
        }
        Ok(())
    }

    /// Only the held records: what a run keeps on disk while vdirsyncer runs, so that a run
    /// that never reaches its update loses every record but those.
    pub fn held_only(&self) -> Manifest {
        Manifest {
            lists: self
                .lists
                .iter()
                .filter(|(c, _)| self.held.contains(*c))
                .map(|(c, f)| (c.clone(), f.clone()))
                .collect(),
            held: self.held.clone(),
        }
    }

    /// Brings the manifest up to date after a run that reached vdirsyncer, from the run's
    /// collection `states`.
    ///
    /// A record is held while its list stays emptied on the server: from a run that reports
    /// it `EmptiedRemote` until a run reports it `Ok` or `Conflict`, through errors, failed
    /// and killed runs. vdirsyncer transfers nothing in an emptied list, and whatever a run
    /// that got further downloads into it is a file the record lacks, which holds the list.
    ///
    /// Otherwise, when the run's reach is unknown (`uncertain`: it timed out or printed
    /// errors of its own), every entry is dropped except those of `EmptiedLocal`
    /// collections. When it is known, `Ok` and `Conflict` collections are re-listed and
    /// `Error` ones dropped: a failed run may have downloaded tasks the old listing lacks.
    /// `EmptiedLocal` and `EmptiedRemote` keep their entry: vdirsyncer stops on an emptied
    /// collection before it transfers anything in it.
    pub fn update(
        &mut self,
        vdir: &Path,
        states: &BTreeMap<String, CollState>,
        uncertain: bool,
    ) -> io::Result<()> {
        self.held = self
            .lists
            .keys()
            .filter(|c| match states.get(*c) {
                Some(CollState::EmptiedRemote) => true,
                Some(CollState::Ok | CollState::Conflict) => false,
                _ => self.held.contains(*c),
            })
            .cloned()
            .collect();
        if uncertain {
            let held = &self.held;
            self.lists
                .retain(|c, _| held.contains(c) || states.get(c) == Some(&CollState::EmptiedLocal));
            return Ok(());
        }
        for (c, s) in states {
            if matches!(s, CollState::Error { .. }) && !self.held.contains(c) {
                self.lists.remove(c);
            }
        }
        self.refresh(
            vdir,
            states
                .iter()
                .filter(|(_, s)| matches!(s, CollState::Ok | CollState::Conflict))
                .map(|(c, _)| c.as_str()),
        )
    }

    /// Whether `pending` (in commit order, as [`crate::store::journal::Journal::pending`]
    /// returns it) accounts for `coll` being empty: the manifest lists at least one file
    /// for it, and the last pending write to each of them removed it. No entry for the
    /// collection means no evidence.
    pub fn emptied_by(&self, coll: &str, pending: &[Entry]) -> bool {
        let Some(files) = self.lists.get(coll) else {
            return false;
        };
        !files.is_empty()
            && files.iter().all(|f| {
                pending
                    .iter()
                    .flat_map(|e| &e.writes)
                    .rfind(|w| &w.path == f)
                    .is_some_and(|w| w.after.is_none())
            })
    }

    /// Whether every `.ics` file now in `vdir/coll` (plain files, not dot-files: the
    /// listing [`Manifest::refresh`] makes) is one the manifest lists for it. `None` when
    /// the manifest has no entry for `coll`. A file the manifest lacks is a task that was
    /// never synced: emptying the list on the server would lose it.
    pub fn covers_local(&self, vdir: &Path, coll: &str) -> io::Result<Option<bool>> {
        let Some(files) = self.lists.get(coll) else {
            return Ok(None);
        };
        if !plain_name(coll) {
            return Ok(Some(false));
        }
        match fs::read_dir(vdir.join(coll)) {
            Ok(dir) => {
                for d in dir {
                    let d = d?;
                    let Some(name) = d.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    if name.starts_with('.') || !name.ends_with(".ics") {
                        continue;
                    }
                    if d.file_type()?.is_file() && !files.contains(&format!("{coll}/{name}")) {
                        return Ok(Some(false));
                    }
                }
                Ok(Some(true))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Some(true)),
            Err(e) => Err(e),
        }
    }
}

fn plain_name(s: &str) -> bool {
    let mut c = Path::new(s).components();
    matches!((c.next(), c.next()), (Some(Component::Normal(_)), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::journal::{Stage, Write};
    use chrono::{TimeZone, Utc};

    fn entry(id: &str, writes: &[(&str, Option<&str>)]) -> Entry {
        Entry {
            format: 1,
            id: id.into(),
            created: Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap(),
            kind: "delete".into(),
            uids: vec![],
            stage: Stage::Applied,
            inverse_of: None,
            from_queue: None,
            request: None,
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: writes
                .iter()
                .map(|(p, a)| Write {
                    path: (*p).into(),
                    before: Some("x".into()),
                    after: a.map(str::to_string),
                })
                .collect(),
        }
    }

    fn manifest(files: &[&str]) -> Manifest {
        let mut m = Manifest::default();
        m.lists
            .insert("c1".into(), files.iter().map(|f| f.to_string()).collect());
        m
    }

    #[test]
    fn every_file_removed_by_the_journal_counts() {
        let m = manifest(&["c1/a.ics", "c1/b.ics"]);
        let p = [
            entry("1", &[("c1/a.ics", None)]),
            entry("2", &[("c1/b.ics", None), ("c2/x.ics", Some("y"))]),
        ];
        assert!(m.emptied_by("c1", &p));
    }

    #[test]
    fn a_file_not_in_the_journal_does_not_count() {
        let m = manifest(&["c1/a.ics", "c1/b.ics", "c1/c.ics"]);
        let p = [entry("1", &[("c1/a.ics", None)])];
        assert!(!m.emptied_by("c1", &p));
    }

    #[test]
    fn a_file_written_again_after_its_removal_does_not_count() {
        let m = manifest(&["c1/a.ics"]);
        let p = [
            entry("1", &[("c1/a.ics", None)]),
            entry("2", &[("c1/a.ics", Some("back"))]),
        ];
        assert!(!m.emptied_by("c1", &p));
    }

    #[test]
    fn no_or_empty_entry_is_no_evidence() {
        let p = [entry("1", &[("c1/a.ics", None)])];
        assert!(!Manifest::default().emptied_by("c1", &p));
        assert!(!manifest(&[]).emptied_by("c1", &p));
    }

    #[test]
    fn update_drops_failed_collections_and_keeps_emptied_ones() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("ok")).unwrap();
        fs::write(vdir.join("ok/n.ics"), "x").unwrap();
        let mut m = Manifest::default();
        for c in ["ok", "err", "el", "er", "other"] {
            m.lists.insert(c.into(), vec![format!("{c}/a.ics")]);
        }
        let states: BTreeMap<String, CollState> = [
            ("ok", CollState::Ok),
            (
                "err",
                CollState::Error {
                    message: "x".into(),
                },
            ),
            ("el", CollState::EmptiedLocal),
            ("er", CollState::EmptiedRemote),
        ]
        .into_iter()
        .map(|(c, s)| (c.to_string(), s))
        .collect();
        let mut clean = m.clone();
        clean.update(&vdir, &states, false).unwrap();
        assert_eq!(
            clean.lists.keys().collect::<Vec<_>>(),
            ["el", "er", "ok", "other"]
        );
        assert_eq!(clean.lists["ok"], vec!["ok/n.ics".to_string()]);
        assert_eq!(clean.lists["el"], vec!["el/a.ics".to_string()]);
        m.update(&vdir, &states, true).unwrap();
        assert_eq!(m.lists.keys().collect::<Vec<_>>(), ["el", "er"]);
    }

    fn states(list: &[(&str, CollState)]) -> BTreeMap<String, CollState> {
        list.iter()
            .map(|(c, s)| (c.to_string(), s.clone()))
            .collect()
    }

    /// Issue #159: a list emptied on the server keeps its record through every run that
    /// does not sync it again, or it could never be applied automatically.
    #[test]
    fn record_of_a_list_emptied_on_the_server_is_held_until_it_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("er")).unwrap();
        fs::write(vdir.join("er/new.ics"), "x").unwrap();
        let mut m = Manifest::default();
        m.lists.insert("er".into(), vec!["er/a.ics".into()]);
        m.lists.insert("ok".into(), vec!["ok/a.ics".into()]);
        let record = vec!["er/a.ics".to_string()];
        let error = CollState::Error {
            message: "x".into(),
        };

        m.update(&vdir, &states(&[("er", CollState::EmptiedRemote)]), false)
            .unwrap();
        assert_eq!(m.held, BTreeSet::from(["er".to_string()]));
        // Uncertain runs: one that reported it emptied, one that never reached it.
        m.update(&vdir, &states(&[("er", CollState::EmptiedRemote)]), true)
            .unwrap();
        m.update(&vdir, &states(&[]), true).unwrap();
        // An error in it, in a run that is otherwise clean, and in an uncertain one.
        m.update(&vdir, &states(&[("er", error.clone())]), false)
            .unwrap();
        m.update(&vdir, &states(&[("er", error)]), true).unwrap();
        assert_eq!(m.lists.keys().collect::<Vec<_>>(), ["er"]);
        assert_eq!(m.lists["er"], record);
        assert_eq!(m.held, BTreeSet::from(["er".to_string()]));

        m.update(&vdir, &states(&[("er", CollState::Ok)]), false)
            .unwrap();
        assert_eq!(m.lists["er"], vec!["er/new.ics".to_string()]);
        assert!(m.held.is_empty());
    }

    #[test]
    fn emptied_on_the_server_without_a_record_holds_nothing() {
        let mut m = Manifest::default();
        m.update(
            Path::new("/nonexistent"),
            &states(&[("er", CollState::EmptiedRemote)]),
            false,
        )
        .unwrap();
        assert_eq!(m, Manifest::default());
    }

    #[test]
    fn held_only_keeps_held_records() {
        let mut m = manifest(&["c1/a.ics"]);
        m.lists.insert("c2".into(), vec!["c2/a.ics".into()]);
        m.held.insert("c2".into());
        let held = m.held_only();
        assert_eq!(held.lists.keys().collect::<Vec<_>>(), ["c2"]);
        assert_eq!(held.held, m.held);
    }

    #[test]
    fn held_round_trips_and_the_plain_format_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manifest(&["c1/a.ics"]);
        m.held.insert("c1".into());
        m.save(dir.path()).unwrap();
        assert_eq!(Manifest::load(dir.path()).unwrap(), m);
        fs::write(
            Manifest::path(dir.path()),
            r#"{"c1": ["c1/a.ics"], "lists": []}"#,
        )
        .unwrap();
        let plain = Manifest::load(dir.path()).unwrap();
        assert_eq!(plain.lists["c1"], vec!["c1/a.ics".to_string()]);
        assert_eq!(plain.lists["lists"], Vec::<String>::new());
        assert!(plain.held.is_empty());
    }

    #[test]
    fn refresh_lists_ics_files_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("c1")).unwrap();
        fs::write(vdir.join("c1/b.ics"), "x").unwrap();
        fs::write(vdir.join("c1/a.ics"), "x").unwrap();
        fs::write(vdir.join("c1/.a.ics.tmp"), "x").unwrap();
        fs::write(vdir.join("c1/displayname"), "x").unwrap();
        let mut m = Manifest::default();
        m.lists.insert("old".into(), vec!["old/z.ics".into()]);
        m.refresh(&vdir, ["c1", "gone", "../x"]).unwrap();
        assert_eq!(
            m.lists["c1"],
            vec!["c1/a.ics".to_string(), "c1/b.ics".to_string()]
        );
        assert_eq!(m.lists["gone"], Vec::<String>::new());
        assert!(!m.lists.contains_key("../x"));
        assert_eq!(m.lists["old"], vec!["old/z.ics".to_string()]);
        let state = dir.path().join("state");
        m.save(&state).unwrap();
        assert_eq!(Manifest::load(&state).unwrap(), m);
        assert_eq!(
            Manifest::load(&dir.path().join("none")).unwrap(),
            Manifest::default()
        );
    }

    #[test]
    fn covers_local_compares_the_directory_with_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path();
        fs::create_dir_all(vdir.join("c1")).unwrap();
        let m = manifest(&["c1/a.ics", "c1/b.ics"]);
        assert_eq!(m.covers_local(vdir, "c2").unwrap(), None);
        fs::write(vdir.join("c1/a.ics"), "x").unwrap();
        assert_eq!(m.covers_local(vdir, "c1").unwrap(), Some(true));
        fs::write(vdir.join("c1/.new.ics.tmp"), "x").unwrap();
        fs::write(vdir.join("c1/.new.ics"), "x").unwrap();
        assert_eq!(m.covers_local(vdir, "c1").unwrap(), Some(true));
        fs::write(vdir.join("c1/new.ics"), "x").unwrap();
        assert_eq!(m.covers_local(vdir, "c1").unwrap(), Some(false));
    }
}
