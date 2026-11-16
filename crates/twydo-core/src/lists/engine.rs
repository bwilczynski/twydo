//! The list operations, run in process: request validation, the guard over the operation
//! record, the phases written to it, and the calls on the server.
//!
//! Every failure is a [`Refused`] with a stable code; nothing here puts server output, an
//! address or a credential into an error.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::call::{Refused, RemoteRequest, wording};
use super::dav::{self, Captured, Changes, Properties, canonical_color, connect, validate_target};
use super::operation::{self, ListOperation, Phase};
use super::{ListError, plain_id};
use crate::config::{Config, SyncConfig};
use crate::store::fsio::{VdirLock, sync_dir, write_atomic};
use crate::sync::run::run_lock;
use crate::sync::vdirsyncer::{self, Outcome, TEST_REMOTE_ENV, Uncontrolled, Vdirsyncer};

/// The words of a refusal as a list error, for callers that report to the user.
pub fn invoke(cfg: &Config, request: &RemoteRequest) -> Result<Value, ListError> {
    run(cfg, request).map_err(|refused| ListError::Refused(wording(refused.code).into()))
}

fn operation_refused() -> Refused {
    Refused::new("operation")
}

/// Local state that cannot be read or written is a stale or mismatched operation.
fn local(_: io::Error) -> Refused {
    operation_refused()
}

/// Whether `id` is a plain collection identity that is safe in a request path.
fn plain(id: &str) -> bool {
    plain_id(id) && id.len() <= 255 && !id.chars().any(|c| "/\\%?#:\0\r\n".contains(c))
}

fn present(path: &Option<PathBuf>) -> bool {
    path.as_ref().is_some_and(|p| !p.as_os_str().is_empty())
}

/// Checks that a request is well formed for its action and mode.
fn validate(r: &RemoteRequest) -> Result<(), Refused> {
    let action = r.action.as_str();
    let mode = r.mode.as_str();
    let operation = present(&r.operation) && present(&r.state);
    let bad = action_unknown(action)
        || r.timeout_secs < 1
        || !matches!(mode, "execute" | "refresh" | "reconcile")
        || (mode != "execute" && !matches!(action, "create" | "edit" | "delete"))
        || (mode == "refresh" && action == "delete")
        || (r.keep_server && mode != "reconcile")
        || !r.vdir.is_absolute()
        || !r.collection.as_deref().is_some_and(plain)
        || (action == "preview-delete" && !operation)
        || (action == "delete" && (!operation || !r.backup.as_deref().is_some_and(plain)))
        || (matches!(action, "create" | "edit") && !operation);
    if bad {
        return Err(operation_refused());
    }
    if matches!(action, "create" | "edit") {
        let name_bad = r.name.as_deref().is_some_and(|n| {
            n.trim().is_empty() || n.len() > 1024 || n.chars().any(|c| c < ' ' || c == '\x7f')
        });
        let color_bad = r.color.as_deref().is_some_and(|c| {
            !(c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()))
        });
        if name_bad
            || color_bad
            || (action == "create" && (r.name.is_none() || r.color.is_none()))
            || (action == "edit"
                && (!r.expected.as_ref().is_some_and(Value::is_object)
                    || (r.name.is_none() && r.color.is_none())))
        {
            return Err(operation_refused());
        }
    }
    Ok(())
}

fn action_unknown(action: &str) -> bool {
    !matches!(
        action,
        "resolve" | "create" | "edit" | "preview-delete" | "delete"
    )
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink())
}

/// The request fields the record's request and the one given must agree on. Absent and null
/// are alike, and the timeout defaults to 300; `mode` and `keep_server` may differ.
fn comparable(request: &impl Serialize) -> Result<BTreeMap<&'static str, Value>, Refused> {
    const FIELDS: [&str; 9] = [
        "action",
        "vdir",
        "collection",
        "operation",
        "state",
        "name",
        "color",
        "expected",
        "backup",
    ];
    let value = serde_json::to_value(request).map_err(|_| operation_refused())?;
    let mut fields = BTreeMap::new();
    for field in FIELDS {
        if let Some(found) = value.get(field).filter(|v| !v.is_null()) {
            fields.insert(field, found.clone());
        }
    }
    fields.insert(
        "timeout_secs",
        value.get("timeout_secs").cloned().unwrap_or(json!(300)),
    );
    Ok(fields)
}

type Files = BTreeMap<String, Vec<u8>>;

/// Whether the local list is what the phase says it may be.
fn local_matches(record: &ListOperation, request: &RemoteRequest, actual: Option<Files>) -> bool {
    let (before, after) = (&record.local_base, &record.local_after);
    match (after, request.mode.as_str()) {
        (Some(after), "refresh") => actual.as_ref() == Some(after),
        (Some(after), "reconcile") => {
            // The finalizer removes scratch files under its exclusive lock. Here only those
            // the persisted write plan justifies are recognized.
            let mut actual = actual.unwrap_or_default();
            let empty = Files::new();
            let before = before.as_ref().unwrap_or(&empty);
            for key in ["displayname", "color"] {
                let temp = format!(".{key}.tdo-tmp");
                let planned = after.get(key);
                let justified = match (planned, actual.get(&temp)) {
                    (Some(planned), Some(data)) => {
                        before.get(key) != Some(planned)
                            && !before.contains_key(&temp)
                            && !record
                                .holds
                                .as_ref()
                                .is_some_and(|holds| holds.contains_key(&temp))
                            && !after.contains_key(&temp)
                            && planned.starts_with(data)
                    }
                    _ => false,
                };
                if justified {
                    actual.remove(&temp);
                }
            }
            actual
                .keys()
                .chain(before.keys())
                .chain(after.keys())
                .all(|key| {
                    let now = actual.get(key);
                    now == before.get(key) || now == after.get(key)
                })
        }
        // Read-only remotely and never writes the vdir: a local edit made after the DELETE
        // must not block learning the outcome.
        (_, "reconcile") if request.action == "delete" => true,
        _ => actual == record.holds,
    }
}

/// The record this request belongs to, checked against the state on disk.
fn guard(
    request: &RemoteRequest,
    state: &Path,
    path: &Path,
    vdir: &Path,
) -> Result<ListOperation, Refused> {
    let marker = state.join("lists/active");
    if is_symlink(&marker) {
        return Err(operation_refused());
    }
    let identity = fs::read_to_string(&marker).map_err(local)?;
    let identity = identity.trim();
    let record = operation::read(state, identity).map_err(local)?;
    let phases: &[Phase] = match request.mode.as_str() {
        "execute" => &[Phase::Prepared],
        "refresh" => &[Phase::RemoteConfirmed],
        _ => &[
            Phase::RemoteStarted,
            Phase::RemoteConfirmed,
            Phase::Uncertain,
        ],
    };
    let bound = fs::read_to_string(state.join(crate::store::VDIR_ID_FILE)).map_err(local)?;
    if path
        != state
            .join("lists/operations")
            .join(format!("{identity}.json"))
            .as_path()
        || comparable(&record.request)? != comparable(request)?
        || !phases.contains(&record.phase)
        || record.vdir != vdir
        || Some(bound.trim()) != vdir.to_str()
    {
        return Err(operation_refused());
    }
    let collection = request.collection.as_deref().unwrap_or_default();
    let actual = operation::snapshot(&vdir.join(collection)).map_err(local)?;
    if !plain(collection) || !local_matches(&record, request, actual) {
        return Err(operation_refused());
    }
    Ok(record)
}

/// The connection an operation started on, so reconciling cannot reach another account.
#[doc(hidden)]
pub fn connection_digest(sync: &SyncConfig, vdir: &Path) -> Result<String, Refused> {
    let encoded = serde_json::to_string(&(
        &sync.url,
        &sync.username,
        &sync.password_command,
        &sync.ca_file,
        vdir,
    ))
    .map_err(|_| Refused::new("mapping"))?;
    Ok(Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `quote(value, safe="")`: everything but unreserved characters, percent-encoded.
fn quote(id: &str) -> String {
    let mut out = String::new();
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

struct Cx<'a> {
    sync: &'a SyncConfig,
    cfg: &'a Config,
    request: &'a RemoteRequest,
    state: Option<&'a Path>,
    /// The canonical vdir.
    vdir: PathBuf,
    deadline: Instant,
}

impl Cx<'_> {
    /// Checked before each server call: past the deadline nothing more is sent.
    fn alive(&self) -> Result<(), Refused> {
        if Instant::now() >= self.deadline {
            return Err(Refused::unsent("timeout"));
        }
        Ok(())
    }

    fn id(&self) -> &str {
        self.request.collection.as_deref().unwrap_or_default()
    }
}

/// The collection's URL among the lists the server reports.
fn select(dav: &dav::Dav, id: &str) -> Result<String, Refused> {
    // Only the list asked for is judged: an unrelated list with an odd id is not this
    // operation's business.
    let mut found = dav.list_home_raw()?.into_iter().filter(|l| l.id == id);
    match (found.next(), found.next()) {
        (Some(only), None) => Ok(only.url),
        _ => Err(Refused::new("mapping")),
    }
}

fn same(color: bool, left: Option<&str>, right: Option<&str>) -> bool {
    if color {
        left.and_then(canonical_color) == right.and_then(canonical_color)
    } else {
        left == right
    }
}

fn differs(changes: &Changes, now: &Properties) -> bool {
    (changes.name.is_some() && !same(false, now.name.as_deref(), changes.name.as_deref()))
        || (changes.color.is_some() && !same(true, now.color.as_deref(), changes.color.as_deref()))
}

fn properties_of(value: &Option<Value>) -> Properties {
    let text = |key: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    Properties {
        name: text("name"),
        color: text("color"),
    }
}

fn read_backup(state: &Path, id: &str) -> Result<Captured, Refused> {
    let directory = state.join("lists/backups").join(id);
    for parent in [
        state.join("lists"),
        state.join("lists/backups"),
        directory.clone(),
    ] {
        let meta = fs::symlink_metadata(&parent).map_err(local)?;
        if meta.is_symlink() || !meta.is_dir() {
            return Err(operation_refused());
        }
    }
    let path = directory.join("remote.json");
    let meta = fs::symlink_metadata(&path).map_err(local)?;
    if meta.is_symlink() || !meta.is_file() {
        return Err(operation_refused());
    }
    let backup: Captured = serde_json::from_slice(&fs::read(&path).map_err(local)?)
        .map_err(|_| operation_refused())?;
    if backup.format != 1 {
        return Err(operation_refused());
    }
    Ok(backup)
}

fn write_backup(state: &Path, id: &str, captured: &Captured) -> Result<(), Refused> {
    let lists = state.join("lists");
    let backups = lists.join("backups");
    let directory = backups.join(id);
    for (dir, parent) in [(&backups, &lists), (&directory, &backups)] {
        operation::private_dir(dir).map_err(local)?;
        sync_dir(parent).map_err(local)?;
    }
    let bytes = serde_json::to_vec_pretty(captured).map_err(|_| operation_refused())?;
    write_atomic(&directory.join("remote.json"), &bytes).map_err(local)
}

/// vdirsyncer's discovery with a policy that refuses any missing collection: it fails on a
/// non-zero exit or an error line, and creates nothing on the server.
fn refresh(cx: &Cx) -> Result<Value, Refused> {
    let discovery = || Refused::new("discovery");
    let state = cx.state.ok_or_else(operation_refused)?;
    let config = vdirsyncer::ensure(cx.cfg, cx.sync, state).map_err(|_| Refused::new("config"))?;
    let ran = Vdirsyncer {
        sync: cx.sync,
        config: &config,
        vdir: &cx.vdir,
        state,
        hook_log: None,
    }
    .discover(&Uncontrolled)
    .map_err(|_| discovery())?;
    let clean = matches!(&ran.outcome, Outcome::Exited(status) if status.success())
        && !ran.stderr.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("error: ") || line.starts_with("critical: ")
        });
    if clean {
        Ok(json!({"refreshed": true}))
    } else {
        Err(discovery())
    }
}

fn save(cx: &Cx, record: &ListOperation) -> Result<(), Refused> {
    let state = cx.state.ok_or_else(operation_refused)?;
    operation::save(state, record).map_err(local)
}

/// Runs the action of one request against the server. Failures before the first request that
/// could write are marked as not sent.
fn dispatch(cx: &Cx, mut record: Option<&mut ListOperation>) -> Result<Value, Refused> {
    let request = cx.request;
    let (mode, action, id) = (request.mode.as_str(), request.action.as_str(), cx.id());
    let unsent = Refused::not_sent;

    // The local list must be in the state the action expects.
    let folder = cx.vdir.join(id);
    let creating = action == "create" && mode == "execute";
    let allow_absent = action == "create" && mode == "reconcile";
    if is_symlink(&folder)
        || (creating && fs::symlink_metadata(&folder).is_ok())
        || (!creating && !allow_absent && !folder.is_dir())
    {
        return Err(Refused::unsent("mapping"));
    }
    if let Some(record) = record.as_deref_mut() {
        let digest = connection_digest(cx.sync, &cx.vdir).map_err(unsent)?;
        if mode == "execute" {
            record.connection = Some(digest);
            save(cx, record).map_err(unsent)?;
        } else {
            // A record without a connection was written by an older Twydo: nothing says
            // which account it started on, so it is never resumed.
            match record.connection.as_ref() {
                None => return Err(Refused::unsent("upgrade")),
                Some(c) if *c != digest => return Err(Refused::unsent("mapping")),
                Some(_) => {}
            }
        }
    }
    if mode == "refresh" {
        return refresh(cx);
    }
    let backup = match (action, cx.state) {
        ("delete", Some(state)) => Some(
            read_backup(state, request.backup.as_deref().unwrap_or_default()).map_err(unsent)?,
        ),
        _ => None,
    };
    let reconciled = if mode == "reconcile" {
        let target = record
            .as_deref()
            .and_then(|r| r.remote_target.clone())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| operation_refused().not_sent())?;
        validate_target(&cx.sync.url, &target, id).map_err(unsent)?;
        Some(target)
    } else {
        None
    };
    cx.alive()?;
    let dav = connect(cx.sync, cx.deadline).map_err(unsent)?;
    let target = match reconciled {
        Some(target) => target,
        None if action == "create" => {
            if record.is_none() {
                return Err(Refused::unsent("operation"));
            }
            cx.alive()?;
            let home = dav.home().map_err(unsent)?;
            format!("{}/{}/", home.trim_end_matches('/'), quote(id))
        }
        None => {
            cx.alive()?;
            select(&dav, id).map_err(unsent)?
        }
    };
    let list = dav.collection(&target, id).map_err(unsent)?;
    if let Some(record) = record.as_deref_mut()
        && mode == "execute"
    {
        record.remote_target = Some(target);
        save(cx, record).map_err(unsent)?;
    }
    cx.alive()?;
    let changes = Changes {
        name: request.name.clone(),
        color: request.color.clone(),
    };
    if action == "delete" {
        let backup = backup.ok_or_else(operation_refused)?;
        if mode == "reconcile" {
            list.reconcile_delete(&backup)?;
        } else {
            list.delete(&backup)?;
        }
        return Ok(json!({"deleted": true, "collection": id}));
    }
    let properties = if mode == "reconcile" {
        let now = list.read_properties()?;
        if !request.keep_server && differs(&changes, &now) {
            if let Some(record) = record.as_deref_mut() {
                record.receipt = Some(json!({"observed": now}));
                save(cx, record)?;
            }
            return Err(Refused::new("choice"));
        }
        now
    } else if action == "create" {
        list.create(
            request.name.as_deref().unwrap_or_default(),
            request.color.as_deref().unwrap_or_default(),
        )?
    } else if action == "edit" {
        list.write_metadata(&changes, &properties_of(&request.expected))?
    } else if action == "preview-delete" {
        let now = list.read_properties()?;
        let captured = list.capture()?;
        let state = cx.state.ok_or_else(operation_refused)?;
        let record_id = record.as_deref().map(|r| r.id.clone()).unwrap_or_default();
        write_backup(state, &record_id, &captured)?;
        return Ok(json!({
            "backup": record_id,
            "resources": captured.resources.len(),
            "name": now.name,
            "color": now.color,
        }));
    } else {
        list.read_properties()?
    };
    Ok(json!({
        "collection": id,
        "remote_collection": id,
        "name": properties.name,
        "color": properties.color,
    }))
}

/// Records how a failed operation ended: uncertain, with the receipt that says what is known.
fn record_failure(cx: &Cx, record: &mut ListOperation, refused: &Refused) {
    let (mode, action, code) = (
        cx.request.mode.as_str(),
        cx.request.action.as_str(),
        refused.code,
    );
    record.phase = Phase::Uncertain;
    if action == "preview-delete" {
        record.receipt = Some(json!({"no_remote_write": true}));
    } else if mode == "execute" && (matches!(code, "stale" | "exists") || !refused.sent) {
        record.receipt = Some(json!({"no_remote_write": true, "reason": code}));
    } else if mode == "reconcile"
        && action == "delete"
        && matches!(code, "present" | "partial" | "changed")
    {
        record.receipt = Some(json!({"present": true, "reason": code}));
    }
    // A record that cannot be written stays at its earlier phase, which also reads as
    // uncertain; the refusal itself is what the caller needs.
    let _ = save(cx, record);
}

/// Runs one list operation: checks the request and the operation record, takes
/// the locks and performs the server calls.
pub fn run(cfg: &Config, request: &RemoteRequest) -> Result<Value, Refused> {
    validate(request)?;
    // A directory standing in for the server is not a CalDAV remote.
    if std::env::var_os(TEST_REMOTE_ENV).is_some_and(|v| !v.is_empty()) {
        return Err(Refused::new("mapping"));
    }
    let sync = cfg.sync.as_ref().ok_or_else(|| Refused::new("config"))?;
    let now = Instant::now();
    let deadline = now
        .checked_add(Duration::from_secs(
            request.timeout_secs.min(365 * 24 * 3600),
        ))
        .unwrap_or_else(|| now + Duration::from_secs(365 * 24 * 3600));
    let vdir = fs::canonicalize(&request.vdir).map_err(local)?;
    let mut cx = Cx {
        sync,
        cfg,
        request,
        state: None,
        vdir: vdir.clone(),
        deadline,
    };
    let Some(path) = request
        .operation
        .as_deref()
        .filter(|p| !p.as_os_str().is_empty())
    else {
        return dispatch(&cx, None);
    };
    let state = request.state.as_deref().unwrap_or(Path::new(""));
    let operations = state.join("lists/operations");
    if !state.is_absolute()
        || is_symlink(state)
        || is_symlink(path)
        || path.parent() != Some(operations.as_path())
        || is_symlink(&operations)
        || is_symlink(&state.join("lists"))
    {
        return Err(operation_refused());
    }
    cx.state = Some(state);
    // The run lock, then the shared vdir lock, both held to the end: remote phases never
    // write the vdir, and a shared lock keeps local readers responsive. Released in reverse.
    operation::private_dir(&state.join("sync")).map_err(local)?;
    let run = run_lock(&state.join("sync"), false).map_err(local)?;
    let shared = VdirLock::shared(state).map_err(local)?;
    let held = (shared, run);
    let mut record = guard(request, state, path, &vdir)?;
    if request.mode == "execute" {
        record.phase = Phase::RemoteStarted;
        save(&cx, &record)?;
    }
    let outcome = dispatch(&cx, Some(&mut record)).and_then(|result| {
        if request.mode == "refresh" {
            record.phase = Phase::LocalConfirmed;
        } else {
            record.phase = Phase::RemoteConfirmed;
            record.receipt = Some(result.clone());
        }
        save(&cx, &record).map(|()| result)
    });
    // A refusal to resume leaves the older record exactly as it was found.
    if let Err(refused) = &outcome
        && refused.code != "upgrade"
    {
        record_failure(&cx, &mut record, refused);
    }
    drop(held);
    outcome
}
