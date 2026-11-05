//! The sync manifest (`<state>/sync/manifest.json`): per collection, the vdir files
//! (`<coll>/<file>.ics`) as of the last run in which that collection counted. It is the
//! evidence the runner needs before it forces an emptied list on its own: only when the
//! journal removed every one of those files did Twydo empty the list.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::status::CollState;
use crate::store::fsio::write_atomic;
use crate::store::journal::Entry;

/// Collection id → its files, sorted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest(pub BTreeMap<String, Vec<String>>);

impl Manifest {
    /// `<state>/sync/manifest.json`.
    pub fn path(state: &Path) -> PathBuf {
        state.join("sync").join("manifest.json")
    }

    /// Reads `<state>/sync/manifest.json`; a missing file is an empty manifest.
    pub fn load(state: &Path) -> io::Result<Manifest> {
        let path = Manifest::path(state);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Manifest)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes `<state>/sync/manifest.json` atomically as pretty JSON.
    pub fn save(&self, state: &Path) -> io::Result<()> {
        fs::create_dir_all(state.join("sync"))?;
        let mut json = serde_json::to_string_pretty(&self.0).map_err(io::Error::other)?;
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
            self.0.insert(coll.to_string(), files);
        }
        Ok(())
    }

    /// Brings the manifest up to date after a run that reached vdirsyncer, from the run's
    /// collection `states`. When the run's reach is unknown (`uncertain`: it timed out or
    /// printed errors of its own), every entry is dropped except those of `EmptiedLocal`
    /// collections. Otherwise `Ok` and `Conflict` collections are re-listed and `Error`
    /// ones dropped: a failed run may have downloaded tasks the old listing lacks.
    /// `EmptiedLocal` and `EmptiedRemote` keep their entry: vdirsyncer stops on an emptied
    /// collection before it transfers anything in it.
    pub fn update(
        &mut self,
        vdir: &Path,
        states: &BTreeMap<String, CollState>,
        uncertain: bool,
    ) -> io::Result<()> {
        if uncertain {
            self.0
                .retain(|c, _| states.get(c) == Some(&CollState::EmptiedLocal));
            return Ok(());
        }
        for (c, s) in states {
            if matches!(s, CollState::Error { .. }) {
                self.0.remove(c);
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

    /// Whether `pending` (sorted by id, as [`crate::store::journal::Journal::pending`]
    /// returns it) accounts for `coll` being empty: the manifest lists at least one file
    /// for it, and the last pending write to each of them removed it. No entry for the
    /// collection means no evidence.
    pub fn emptied_by(&self, coll: &str, pending: &[Entry]) -> bool {
        let Some(files) = self.0.get(coll) else {
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
            synced: None,
            holds: Vec::new(),
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
        m.0.insert("c1".into(), files.iter().map(|f| f.to_string()).collect());
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
            m.0.insert(c.into(), vec![format!("{c}/a.ics")]);
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
            clean.0.keys().collect::<Vec<_>>(),
            ["el", "er", "ok", "other"]
        );
        assert_eq!(clean.0["ok"], vec!["ok/n.ics".to_string()]);
        assert_eq!(clean.0["el"], vec!["el/a.ics".to_string()]);
        m.update(&vdir, &states, true).unwrap();
        assert_eq!(m.0.keys().collect::<Vec<_>>(), ["el"]);
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
        m.0.insert("old".into(), vec!["old/z.ics".into()]);
        m.refresh(&vdir, ["c1", "gone", "../x"]).unwrap();
        assert_eq!(
            m.0["c1"],
            vec!["c1/a.ics".to_string(), "c1/b.ics".to_string()]
        );
        assert_eq!(m.0["gone"], Vec::<String>::new());
        assert!(!m.0.contains_key("../x"));
        assert_eq!(m.0["old"], vec!["old/z.ics".to_string()]);
        let state = dir.path().join("state");
        m.save(&state).unwrap();
        assert_eq!(Manifest::load(&state).unwrap(), m);
        assert_eq!(
            Manifest::load(&dir.path().join("none")).unwrap(),
            Manifest::default()
        );
    }
}
