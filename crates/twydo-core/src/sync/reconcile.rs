//! Reconciliation after a vdirsyncer run: confirm journal entries whose result the server
//! now holds, turn silently settled delete-versus-edit cases (F6) into conflicts, close held
//! conflicts the user decided (or that both sides now agree on), and prune old records.
//!
//! Runs under the exclusive vdir lock, which the caller holds. It takes the conflicts lock
//! (inside [`ConflictStore`]) after it, never the other way round.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};

use super::hook::{HookRecord, uid_of};
use super::output::{Action, SyncOutput};
use super::status::CollState;
use crate::config::SyncConfig;
use crate::conflicts::{Conflict, ConflictKind, ConflictState, ConflictStore, NewConflict, Side};
use crate::store::journal::{Entry, Journal, Stage};
use crate::store::queue::Queue;

/// What [`reconcile`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reconciled {
    /// Journal entries marked synced.
    pub confirmed: Vec<String>,
    /// Delete-versus-edit conflicts recorded.
    pub new_conflicts: Vec<String>,
    /// Held conflicts marked resolved.
    pub resolved: Vec<String>,
    /// Paths whose vdir file differs from what Twydo wrote while it may not hold the
    /// server's version yet (it still holds what Twydo replaced, or a conflict is open for
    /// it); their entries stay pending.
    pub unexpected: Vec<String>,
    /// Paths the server changed again after Twydo's edit reached it, seen too late to tell
    /// for sure: confirmed with the server's version, reported once.
    pub superseded: Vec<String>,
    /// Journal entries and conflicts pruned.
    pub pruned: usize,
}

/// How one path came out.
enum Outcome {
    /// Not decided this run; its entries stay pending.
    Pending,
    /// The server holds Twydo's result, or a conflict now carries it, or the user decided.
    Done,
}

/// Reconciles the journal with one vdirsyncer run (see the module docs).
///
/// Step 4 below runs first for every path written by `pending`, whatever the state of its
/// collection and even when `out.unattributed` is non-empty, so a re-upload in a failed run
/// still records its conflict. Otherwise only collections that are `Ok` or `Conflict` in
/// `states` count, and nothing counts when `out.unattributed` is non-empty. For each path
/// written by `pending` in a counted collection, in write order:
///
/// 1. skip it when a journal entry not in `pending` touches it (a write that slipped in
///    between the two locks; it is checked next run);
/// 2. the expected content is the last pending write's `after`; the path was created here
///    when a pending write has `before: None`; the uid comes from the expected content, or
///    from the last `before` for a deletion;
/// 3. an unresolved held conflict for the path whose collection is still `Conflict` keeps it
///    pending;
/// 4. an upload of the uid to the remote side, for a path not created here, is a remote
///    delete versus a local edit (F6);
/// 5. a file that differs from the expected content is a local delete versus a remote edit
///    when the expected content is a deletion; it is confirmed when it differs only in line
///    endings or vdirsyncer brought it down this run; it stays pending when it still holds a
///    pending write's `before` or an unresolved conflict exists for the path (see
///    [`Reconciled::unexpected`]); otherwise it is confirmed with a warning (see
///    [`Reconciled::superseded`]);
/// 6. otherwise the path is confirmed.
///
/// An entry is marked synced when every path it writes is done. Held conflicts with a
/// `resolved` hook record, or with no hook record while their collection is `Ok` (both
/// sides agree, F5), are resolved, and their path counts as done. Finally journal entries
/// and conflicts older than `retention_days` are pruned.
#[allow(clippy::too_many_arguments)]
pub fn reconcile(
    vdir: &Path,
    journal: &Journal,
    conflicts: &ConflictStore,
    sync: &SyncConfig,
    states: &BTreeMap<String, CollState>,
    pending: &[Entry],
    out: &SyncOutput,
    hook: &[HookRecord],
    now: DateTime<Utc>,
) -> io::Result<Reconciled> {
    let mut rec = Reconciled::default();
    // Step 4 counts whatever the collection's state: the upload line is the only trace of
    // the phone's deletion, and the next clean run would confirm the entry without it.
    remote_deletes(vdir, conflicts, sync, pending, out, now, &mut rec)?;
    if out.unattributed.is_empty() {
        settle(
            vdir, journal, conflicts, sync, states, pending, out, hook, now, &mut rec,
        )?;
    }
    // A retention too long to subtract keeps everything.
    if let Some(before) =
        now.checked_sub_signed(chrono::Duration::days(i64::from(sync.retention_days)))
    {
        rec.pruned = journal.prune(before)?.len() + conflicts.prune(before)?.len();
        if let Some(queue) = Queue::existing(journal.state_dir()) {
            rec.pruned += queue.prune(before)?;
        }
    }
    Ok(rec)
}

/// Everything but pruning; only called when the run's output is fully attributed.
#[allow(clippy::too_many_arguments)]
fn settle(
    vdir: &Path,
    journal: &Journal,
    conflicts: &ConflictStore,
    sync: &SyncConfig,
    states: &BTreeMap<String, CollState>,
    pending: &[Entry],
    out: &SyncOutput,
    hook: &[HookRecord],
    now: DateTime<Utc>,
    rec: &mut Reconciled,
) -> io::Result<()> {
    let state_of = |coll: &str| states.get(coll);
    let counted = |coll: &str| matches!(state_of(coll), Some(CollState::Ok | CollState::Conflict));

    let all_conflicts = conflicts.list()?;
    // Held conflicts the user decided, or that both sides now agree on. Their paths are
    // done: whatever the vdir holds now is the user's decision.
    let held: Vec<&Conflict> = all_conflicts
        .iter()
        .filter(|c| c.held && matches!(c.state, ConflictState::Open | ConflictState::Chosen))
        .collect();
    let mut closing: Vec<&Conflict> = Vec::new();
    let mut still_held: BTreeSet<&str> = BTreeSet::new();
    for c in &held {
        if !counted(&c.collection) {
            continue;
        }
        let records: Vec<&HookRecord> = hook
            .iter()
            .filter(|r| r.conflict == c.id || (r.uid == c.uid && r.collection == c.collection))
            .collect();
        let unhooked = out
            .unhooked
            .get(&c.collection)
            .is_some_and(|uids| uids.contains(&c.uid));
        if records.iter().any(|r| r.conflict == c.id && r.resolved) {
            // The hook applied the user's choice.
            closing.push(c);
        } else if !records.is_empty() || unhooked {
            // vdirsyncer still sees the item changed on both sides (step 3).
            still_held.insert(&c.path);
        } else if agreed(vdir, c, out) {
            closing.push(c);
        }
        // Otherwise vdirsyncer settled the item some other way (e.g. a delete versus an
        // edit, F6): the path goes through the normal checks, which supersede the conflict
        // if they record a new one.
    }
    let decided: BTreeSet<&str> = closing.iter().map(|c| c.path.as_str()).collect();

    let order = write_order(pending);
    let ours: HashSet<&str> = pending.iter().map(|e| e.id.as_str()).collect();
    // Entries written since `pending` was read (step 1).
    let others: Vec<Entry> = journal
        .list()?
        .into_iter()
        .filter(|e| !ours.contains(e.id.as_str()))
        .filter(|e| e.synced.is_none() && e.stage != Stage::Abandoned)
        .collect();

    let paths = written_paths(&order);

    let mut done: BTreeSet<&str> = BTreeSet::new();
    for path in paths {
        let Some(coll) = collection_of(path) else {
            continue;
        };
        if !counted(coll) {
            continue;
        }
        // Step 1.
        if others
            .iter()
            .any(|e| e.writes.iter().any(|w| w.path == path))
        {
            continue;
        }
        if decided.contains(path) {
            done.insert(path);
            continue;
        }
        // Step 3.
        if still_held.contains(path) {
            continue;
        }
        let outcome = check_path(
            vdir,
            conflicts,
            &all_conflicts,
            sync,
            out,
            &order,
            path,
            coll,
            now,
            rec,
        )?;
        if let Outcome::Done = outcome {
            done.insert(path);
        }
    }

    // Journal first, then conflicts: a crash in between leaves an open conflict, which the
    // next run closes again, rather than an entry nothing can confirm any more.
    for e in &order {
        if !e.writes.iter().all(|w| done.contains(w.path.as_str())) {
            continue;
        }
        // Only an Applied entry not yet confirmed (`mark_synced` does not check).
        match journal.get(&e.id)? {
            Some(cur) if cur.stage == Stage::Applied && cur.synced.is_none() => {}
            _ => continue,
        }
        journal.mark_synced(&e.id, now)?;
        rec.confirmed.push(e.id.clone());
    }
    for c in closing {
        conflicts.mark_resolved(&c.id, now)?;
        rec.resolved.push(c.id.clone());
    }
    Ok(())
}

/// Steps 2 and 4 to 6 for one path.
#[allow(clippy::too_many_arguments)]
fn check_path(
    vdir: &Path,
    conflicts: &ConflictStore,
    known: &[Conflict],
    sync: &SyncConfig,
    out: &SyncOutput,
    order: &[&Entry],
    path: &str,
    coll: &str,
    now: DateTime<Utc>,
    rec: &mut Reconciled,
) -> io::Result<Outcome> {
    // Step 2.
    let Some(f) = facts(vdir, order, path) else {
        return Ok(Outcome::Pending);
    };
    let (last_entry, expected, uid, file) =
        (f.last_entry, f.expected, f.uid.clone(), f.file.clone());
    let transferred = |action: Action, to_local: bool| {
        uid.as_deref()
            .is_some_and(|uid| transferred(out, sync, coll, uid, action, to_local))
    };
    let mut record = |c: NewConflict| -> io::Result<Outcome> {
        record_once(conflicts, known, last_entry, c, now, rec)?;
        Ok(Outcome::Done)
    };

    // Step 4.
    if let Some(c) = remote_delete(&f, out, sync, coll, path) {
        return record(c);
    }

    // Step 5.
    let mut current = read_opt(&file)?;
    let mut at = path.to_string();
    if expected.is_none() && current.is_none() && transferred(Action::Upload, true) {
        // vdirsyncer restored the remote edit under a file name of its own. If no single
        // file holds it, the deletion must not pass as confirmed.
        match find_uid(vdir, coll, uid.as_deref().unwrap_or_default())? {
            Some((p, text)) => {
                at = p;
                current = Some(text);
            }
            None => {
                rec.unexpected.push(path.to_string());
                return Ok(Outcome::Pending);
            }
        }
    }
    if current.as_deref() == expected {
        // Step 6.
        return Ok(Outcome::Done);
    }
    let Some(expected) = expected else {
        // Local delete versus remote edit: vdirsyncer restored the remote edit (F6).
        return record(NewConflict {
            uid: uid.unwrap_or_default(),
            collection: coll.to_string(),
            path: at,
            kind: ConflictKind::LocalDeleteRemoteEdit,
            held: false,
            vdir_holds: Side::Remote,
            local: None,
            remote: current,
        });
    };
    if let Some(cur) = &current {
        // vdirsyncer may rewrite an item with LF line endings (it reads items in text
        // mode), so a file that differs only in line endings holds Twydo's result: the
        // server has it. Confirmed.
        if same_text(cur, expected) {
            return Ok(Outcome::Done);
        }
        // vdirsyncer overwrote the local file without a conflict, which it only does when
        // the local side was unchanged since its last sync: Twydo's edit reached the server
        // in an earlier run (e.g. vdirsyncer run by hand) and the server changed it since.
        if transferred(Action::Update, true) || transferred(Action::Upload, true) {
            return Ok(Outcome::Done);
        }
    }
    // The server changed the item after Twydo's edit reached it, in a run this one did not
    // see (vdirsyncer by hand, or an entry written before sync was enabled): the file holds
    // neither Twydo's result nor what it replaced, and no conflict explains it. Nothing
    // later can confirm it, so confirm it now, with a warning.
    let unchanged = current
        .as_deref()
        .is_some_and(|cur| f.befores.iter().any(|b| same_text(cur, b)));
    let explained = known
        .iter()
        .any(|k| k.path == path && matches!(k.state, ConflictState::Open | ConflictState::Chosen));
    if !unchanged && !explained {
        rec.superseded.push(path.to_string());
        return Ok(Outcome::Done);
    }
    rec.unexpected.push(path.to_string());
    Ok(Outcome::Pending)
}

/// Pending entries in write order: by creation time, then id (ids within one second are
/// random).
fn write_order(pending: &[Entry]) -> Vec<&Entry> {
    let mut order: Vec<&Entry> = pending.iter().collect();
    order.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    order
}

/// Every path `order` writes, first write first.
fn written_paths<'a>(order: &[&'a Entry]) -> Vec<&'a str> {
    let mut paths: Vec<&str> = Vec::new();
    for e in order {
        for w in &e.writes {
            if !paths.contains(&w.path.as_str()) {
                paths.push(&w.path);
            }
        }
    }
    paths
}

/// Step 2 for one path: what the pending writes to it say.
struct Facts<'a> {
    /// The entry of the last pending write to the path.
    last_entry: &'a Entry,
    /// The last write's `after`: what the server should now hold.
    expected: Option<&'a str>,
    /// The `before` of every pending write to the path.
    befores: Vec<&'a str>,
    created_here: bool,
    uid: Option<String>,
    file: std::path::PathBuf,
}

fn facts<'a>(vdir: &Path, order: &[&'a Entry], path: &str) -> Option<Facts<'a>> {
    let writes: Vec<(&Entry, &crate::store::journal::Write)> = order
        .iter()
        .flat_map(|e| e.writes.iter().map(move |w| (*e, w)))
        .filter(|(_, w)| w.path == path)
        .collect();
    let &(last_entry, last) = writes.last()?;
    let expected = last.after.as_deref();
    let file = vdir.join(path);
    let uid = expected
        .or(last.before.as_deref())
        .and_then(|t| uid_of(t, &file).ok())
        .or_else(|| match last_entry.uids.as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        });
    Some(Facts {
        last_entry,
        expected,
        befores: writes
            .iter()
            .filter_map(|(_, w)| w.before.as_deref())
            .collect(),
        created_here: writes.iter().any(|(_, w)| w.before.is_none()),
        uid,
        file,
    })
}

/// Whether `out` has an `action` of `uid` in `coll`, to the local storage or not.
fn transferred(
    out: &SyncOutput,
    sync: &SyncConfig,
    coll: &str,
    uid: &str,
    action: Action,
    to_local: bool,
) -> bool {
    out.transfers.iter().any(|t| {
        t.action == action
            && t.uid == uid
            && t.collection == coll
            && (t.storage == sync.local_storage) == to_local
    })
}

/// Step 4: remote delete versus local edit. vdirsyncer re-uploaded Twydo's edit to a
/// server that had deleted the item (F6). A path created here is skipped: its upload is the
/// create itself. That also skips a create uploaded in an earlier run that did not count,
/// edited since and deleted on the other device (a documented limit, docs/sync.md).
fn remote_delete(
    f: &Facts,
    out: &SyncOutput,
    sync: &SyncConfig,
    coll: &str,
    path: &str,
) -> Option<NewConflict> {
    let local = f.expected?;
    let uid = f.uid.as_deref()?;
    if f.created_here || !transferred(out, sync, coll, uid, Action::Upload, false) {
        return None;
    }
    Some(NewConflict {
        uid: uid.to_string(),
        collection: coll.to_string(),
        path: path.to_string(),
        kind: ConflictKind::RemoteDeleteLocalEdit,
        held: false,
        vdir_holds: Side::Local,
        local: Some(local.to_string()),
        remote: None,
    })
}

/// Records `c` unless one like it was recorded for the pending write already (resolved or
/// not): a later run that re-checks the path (the entry still pending for another path, or
/// the first record made in a run that did not count) neither reports nor re-creates it.
fn record_once(
    conflicts: &ConflictStore,
    known: &[Conflict],
    last_entry: &Entry,
    c: NewConflict,
    now: DateTime<Utc>,
    rec: &mut Reconciled,
) -> io::Result<()> {
    let seen = known.iter().any(|k| {
        k.path == c.path
            && k.kind == c.kind
            && k.local == c.local
            && k.remote == c.remote
            && k.detected >= last_entry.created
    });
    if !seen {
        rec.new_conflicts.push(conflicts.record(c, now)?.id);
    }
    Ok(())
}

/// Step 4 for every path `pending` writes, whatever the state of its collection. Marks
/// nothing synced: [`settle`] does that, for counted collections only, and finds the
/// conflict recorded here already.
fn remote_deletes(
    vdir: &Path,
    conflicts: &ConflictStore,
    sync: &SyncConfig,
    pending: &[Entry],
    out: &SyncOutput,
    now: DateTime<Utc>,
    rec: &mut Reconciled,
) -> io::Result<()> {
    if !out
        .transfers
        .iter()
        .any(|t| t.action == Action::Upload && t.storage != sync.local_storage)
    {
        return Ok(());
    }
    let known = conflicts.list()?;
    let order = write_order(pending);
    for path in written_paths(&order) {
        let Some(coll) = collection_of(path) else {
            continue;
        };
        let Some(f) = facts(vdir, &order, path) else {
            continue;
        };
        if let Some(c) = remote_delete(&f, out, sync, coll, path) {
            record_once(conflicts, &known, f.last_entry, c, now, rec)?;
        }
    }
    Ok(())
}

/// Whether held conflict `c`, with no hook call this run, is settled because both sides
/// now agree. Needs positive evidence, because a delete versus an edit is settled silently
/// too (F6): no transfer of the item this run, and either vdirsyncer reported the same
/// content on both sides (F5) or the vdir file holds one of the two versions (ignoring line
/// endings: a choice applied by a vdirsyncer run without a hook log comes back with LF), or
/// the file is gone and no file of the collection holds the uid (both sides deleted it).
fn agreed(vdir: &Path, c: &Conflict, out: &SyncOutput) -> bool {
    let moved = out
        .transfers
        .iter()
        .any(|t| t.uid == c.uid && t.collection == c.collection);
    if moved {
        return false;
    }
    if out.conflict_items.contains(&c.uid) {
        return true;
    }
    let current = match read_opt(&vdir.join(&c.path)) {
        Ok(Some(current)) => current,
        // Gone here, not restored, and no other file holds it: both sides deleted it.
        Ok(None) => {
            return !c.uid.is_empty()
                && uid_files(vdir, &c.collection, &c.uid).is_ok_and(|n| n.is_empty());
        }
        Err(_) => return false,
    };
    [&c.local, &c.remote]
        .into_iter()
        .flatten()
        .any(|v| same_text(&current, v))
}

/// The collection of a vdir-relative path: its first component, when the path is
/// `<collection>/<file>`.
fn collection_of(path: &str) -> Option<&str> {
    match path.split_once('/') {
        Some((coll, file)) if !coll.is_empty() && !file.is_empty() && !file.contains('/') => {
            Some(coll)
        }
        _ => None,
    }
}

/// Equal up to CRLF versus LF line endings.
pub(crate) fn same_text(a: &str, b: &str) -> bool {
    a == b || a.replace("\r\n", "\n") == b.replace("\r\n", "\n")
}

fn read_opt(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The single `.ics` file of `coll` holding `uid`, as (vdir-relative path, content).
fn find_uid(vdir: &Path, coll: &str, uid: &str) -> io::Result<Option<(String, String)>> {
    let mut found = uid_files(vdir, coll, uid)?;
    Ok(if found.len() == 1 { found.pop() } else { None })
}

/// Every `.ics` file of `coll` holding `uid`, as (vdir-relative path, content). A missing
/// collection directory holds none.
fn uid_files(vdir: &Path, coll: &str, uid: &str) -> io::Result<Vec<(String, String)>> {
    let mut found = Vec::new();
    if uid.is_empty() {
        return Ok(found);
    }
    let dir = match fs::read_dir(vdir.join(coll)) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(e),
    };
    for d in dir {
        let p = d?.path();
        if p.extension().is_none_or(|x| x != "ics") {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(text) = read_opt(&p)? else { continue };
        if uid_of(&text, &p).is_ok_and(|u| u == uid) {
            found.push((format!("{coll}/{name}"), text));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_of_paths() {
        assert_eq!(collection_of("c1/a.ics"), Some("c1"));
        assert_eq!(collection_of("a.ics"), None);
        assert_eq!(collection_of("c1/x/a.ics"), None);
        assert_eq!(collection_of("/a.ics"), None);
    }

    #[test]
    fn same_text_ignores_line_endings_only() {
        assert!(same_text("a\r\nb\r\n", "a\nb\n"));
        assert!(same_text("a\nb\n", "a\nb\n"));
        assert!(!same_text("a\r\nb\r\n", "a\nc\n"));
    }
}
