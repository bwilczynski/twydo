//! Collection markers must fence task writes, queueing and ordinary sync.
use chrono::Utc;
use std::fs;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::lists::{
    helper::HelperRequest,
    operation::{self, Phase},
};
use twydo_core::store::{Store, journal::Journal};

fn setup() -> (tempfile::TempDir, Config, HelperRequest) {
    let root = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(root.path().join("vdir"));
    cfg.state_dir = Some(root.path().join("state"));
    cfg.sync = Some(SyncConfig::default());
    fs::create_dir_all(cfg.vdir.join("synthetic")).unwrap();
    fs::write(cfg.vdir.join("synthetic/displayname"), "Synthetic").unwrap();
    let mut request = HelperRequest::probe(&cfg).unwrap();
    request.action = "resolve".into();
    request.collection = Some("synthetic".into());
    (root, cfg, request)
}

#[test]
fn unfinished_operation_fences_recovery_and_sync_without_running_a_command() {
    let (_root, cfg, request) = setup();
    let record = operation::prepare(&cfg, request, Utc::now()).unwrap();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(operation::ensure_idle(&state).is_err());
    assert!(Store::open(&cfg).is_err());
    let sync = twydo_core::sync::run::run(&cfg, &Default::default(), Utc::now());
    assert!(sync.is_err());
    assert!(Journal::open(&state).unwrap().list().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/displayname")).unwrap(),
        "Synthetic"
    );
    assert_eq!(record.phase, Phase::Prepared);
}

#[test]
fn stale_collection_bytes_cannot_be_confirmed_or_silently_rebased() {
    let (_root, cfg, request) = setup();
    let mut record = operation::prepare(&cfg, request, Utc::now()).unwrap();
    fs::write(
        cfg.vdir.join("synthetic/new.ics"),
        "synthetic concurrent task",
    )
    .unwrap();
    assert!(operation::validate_local(&record).is_err());
    assert!(operation::finish(&cfg, &mut record).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(operation::ensure_idle(&state).is_err());
}

#[test]
fn operation_requires_explicit_remote_and_local_confirmation() {
    let (_root, cfg, request) = setup();
    let mut record = operation::prepare(&cfg, request, Utc::now()).unwrap();
    assert!(operation::finish(&cfg, &mut record).is_err());
    operation::set_phase(&cfg, &mut record, Phase::RemoteStarted).unwrap();
    operation::set_phase(&cfg, &mut record, Phase::RemoteConfirmed).unwrap();
    operation::set_phase(&cfg, &mut record, Phase::LocalConfirmed).unwrap();
    operation::finish(&cfg, &mut record).unwrap();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    operation::ensure_idle(&state).unwrap();
    assert!(Store::open(&cfg).is_ok());
}

#[test]
fn malformed_active_marker_fails_closed() {
    let (_root, cfg, request) = setup();
    operation::prepare(&cfg, request, Utc::now()).unwrap();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    fs::write(state.join("lists/active"), "../other").unwrap();
    assert!(operation::ensure_idle(&state).is_err());
}

#[test]
fn unreadable_queue_prevents_preparing_a_collection_operation() {
    let (_root, cfg, request) = setup();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    fs::create_dir_all(state.join("queue")).unwrap();
    fs::write(state.join("queue/interrupted.json"), "{").unwrap();
    assert!(operation::prepare(&cfg, request, Utc::now()).is_err());
    assert!(!state.join("lists/active").exists());
}

#[cfg(unix)]
#[test]
fn helper_owns_both_locks_independently_of_the_rust_caller() {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let (_root, cfg, request) = setup();
    let operation = operation::prepare(&cfg, request, Utc::now()).unwrap();
    let source = serde_json::to_string(include_str!("../src/lists/helper.py")).unwrap();
    let code = format!(
        "import json,sys,time\nns={{'__name__':'helper'}}\nexec({source},ns)\nwith ns['operation_guard'](json.loads(sys.stdin.read())):\n print('locked',flush=True)\n time.sleep(5)\n"
    );
    let mut child = Command::new("python3")
        .args(["-I", "-c", &code])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&operation.request).unwrap())
        .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "locked");
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let vdir_held = twydo_core::store::fsio::VdirLock::try_exclusive(&state)
        .unwrap()
        .is_none();
    let run = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("sync/run.lock"))
        .unwrap();
    let run_held = matches!(run.try_lock(), Err(std::fs::TryLockError::WouldBlock));
    let reader = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("vdir.lock"))
        .unwrap();
    let reader_allowed = reader.try_lock_shared().is_ok();
    drop(reader);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(vdir_held && run_held);
    assert!(
        reader_allowed,
        "readers must remain responsive during the remote-only phase"
    );
    assert!(
        twydo_core::store::fsio::VdirLock::try_exclusive(&state)
            .unwrap()
            .is_some()
    );
}

#[cfg(unix)]
#[test]
fn helper_retains_locks_after_its_launcher_exits() {
    use std::process::Command;
    let (root, cfg, request) = setup();
    let operation = operation::prepare(&cfg, request, Utc::now()).unwrap();
    let source = serde_json::to_string(include_str!("../src/lists/helper.py")).unwrap();
    let payload =
        serde_json::to_string(&serde_json::to_string(&operation.request).unwrap()).unwrap();
    let ready = root.path().join("ready");
    let release = root.path().join("release");
    let code = format!(
        "import json,time\nfrom pathlib import Path\nns={{'__name__':'helper'}}\nexec({source},ns)\nwith ns['operation_guard'](json.loads({payload})):\n Path({ready:?}).write_text('locked')\n deadline=time.monotonic()+8\n while not Path({release:?}).exists() and time.monotonic()<deadline: time.sleep(0.01)\n"
    );
    let code = serde_json::to_string(&code).unwrap();
    let launcher = format!(
        "import subprocess,sys\nsubprocess.Popen([sys.executable,'-I','-c',{code}],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)\n"
    );
    assert!(
        Command::new("python3")
            .args(["-I", "-c", &launcher])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let held = twydo_core::store::fsio::VdirLock::try_exclusive(&state)
        .unwrap()
        .is_none();
    fs::write(release, "release").unwrap();
    assert!(
        ready.exists() && held,
        "helper did not retain its lock after its launcher exited"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while twydo_core::store::fsio::VdirLock::try_exclusive(&state)
        .unwrap()
        .is_none()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "helper did not release its locks"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The fence moved from `Queue::push` to the drain (daemon plan 2): an edit queued while a
/// list operation runs is written to the queue, and a drain during the operation refuses an
/// item of the fenced list with the fence's message; unrelated queued work waits.
#[test]
fn active_marker_fences_queued_edits_at_drain_and_preserves_unrelated_queued_work() {
    use twydo_core::edit::{NewTask, Op};
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock, queue::Queue};
    use twydo_core::views::Snapshot;
    let (_root, cfg, request) = setup();
    fs::create_dir(cfg.vdir.join("other")).unwrap();
    let view = Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone());
    let (mut store, _) = Store::open(&cfg).unwrap();
    let state = store.state_dir().to_path_buf();
    let queue = Queue::open(&state).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    let mut queued = |list: &str| {
        let submitted = store
            .submit(
                &view,
                &Op::Create(NewTask::plain(list, "Synthetic task")),
                "create",
                "test",
                Utc::now(),
            )
            .unwrap();
        let Submitted::Queued(item) = submitted else {
            panic!("expected queued create")
        };
        item
    };
    let item = queued("other");
    // An edit of the fenced list, as one saved during the list job: planned before the
    // operation, queued after it began.
    let fenced = queued("synthetic");
    assert!(queue.cancel(&fenced.id).unwrap());
    drop(lock);
    let mut operation = operation::prepare(&cfg, request, Utc::now()).unwrap();
    queue.push(&fenced).unwrap();
    assert!(
        store
            .submit(
                &view,
                &Op::Create(NewTask::plain("synthetic", "Another task")),
                "create",
                "test",
                Utc::now()
            )
            .is_err(),
        "an edit saved outside a list job is still fenced at once"
    );
    let drained = store.drain(Utc::now()).unwrap();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].item.id, fenced.id);
    let DrainOutcome::Refused(reason) = &drained[0].outcome else {
        panic!("expected a refusal: {:?}", drained[0].outcome)
    };
    assert!(
        reason.contains("is unfinished; reconcile it before writing or syncing"),
        "{reason}"
    );
    assert_eq!(queue.list().unwrap(), vec![item.clone()]);
    let error = store.drain(Utc::now()).unwrap_err().to_string();
    assert!(error.contains("is unfinished"), "{error}");
    operation::set_phase(&cfg, &mut operation, Phase::RemoteStarted).unwrap();
    operation::set_phase(&cfg, &mut operation, Phase::RemoteConfirmed).unwrap();
    operation::set_phase(&cfg, &mut operation, Phase::LocalConfirmed).unwrap();
    operation::finish(&cfg, &mut operation).unwrap();
    assert_eq!(queue.list().unwrap(), vec![item]);
    assert_eq!(store.drain(Utc::now()).unwrap().len(), 1);
    assert_eq!(Journal::open(&state).unwrap().pending().unwrap().len(), 1);
}

/// Undo and redo never plan through the gate, so a queued one is fenced when it is saved:
/// with an operation active and the vdir held, both are refused at once.
#[test]
fn queued_undo_and_redo_keep_the_save_time_fence() {
    use twydo_core::edit::{NewTask, Op};
    use twydo_core::store::{Submitted, fsio::VdirLock};
    use twydo_core::views::Snapshot;
    let (_root, cfg, request) = setup();
    fs::create_dir(cfg.vdir.join("other")).unwrap();
    let view = Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone());
    let (mut store, _) = Store::open(&cfg).unwrap();
    let state = store.state_dir().to_path_buf();
    let create = |title: &str| Op::Create(NewTask::plain("other", title));
    // An undo of an applied step.
    let applied = store
        .submit(&view, &create("Applied"), "create", "test", Utc::now())
        .unwrap();
    assert!(matches!(applied, Submitted::Applied(_)));
    // A redo of a cancelled step: queued while the vdir is held, then undone (cancelled).
    let lock = VdirLock::exclusive(&state).unwrap();
    let queued = store
        .submit(&view, &create("Queued"), "create", "test", Utc::now())
        .unwrap();
    assert!(matches!(queued, Submitted::Queued(_)));
    assert!(matches!(
        store.undo_last(Utc::now()).unwrap(),
        Submitted::Cancelled
    ));
    drop(lock);
    let _operation = operation::prepare(&cfg, request, Utc::now()).unwrap();
    let _lock = VdirLock::exclusive(&state).unwrap();
    for (what, got) in [
        ("undo", store.undo_last(Utc::now())),
        // The redo stack holds the cancelled create.
        ("redo", store.redo_last(Utc::now())),
    ] {
        let error = got.unwrap_err().to_string();
        assert!(
            error.contains("is unfinished; reconcile it before writing or syncing"),
            "{what}: {error}"
        );
    }
    assert!(
        twydo_core::store::queue::Queue::open(&state)
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn metadata_finalization_preserves_tasks_and_is_repeatable_after_partial_writes() {
    let (_root, cfg, request) = setup();
    fs::write(cfg.vdir.join("synthetic/held.ics"), "held synthetic bytes").unwrap();
    let mut record = operation::prepare(&cfg, request, Utc::now()).unwrap();
    operation::set_phase(&cfg, &mut record, Phase::RemoteStarted).unwrap();
    record.receipt = Some(serde_json::json!({"name":"After", "color":"#379417"}));
    operation::set_phase(&cfg, &mut record, Phase::RemoteConfirmed).unwrap();
    operation::finalize_metadata(&cfg, &mut record).unwrap();
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/displayname")).unwrap(),
        "After"
    );
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/color")).unwrap(),
        "#379417"
    );
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/held.ics")).unwrap(),
        "held synthetic bytes"
    );
    // An interruption can leave either the old or new metadata file; both are recorded.
    fs::write(cfg.vdir.join("synthetic/displayname"), "Synthetic").unwrap();
    let temp = cfg.vdir.join("synthetic/.displayname.tdo-tmp");
    fs::write(&temp, "Af").unwrap();
    operation::finalize_metadata(&cfg, &mut record).unwrap();
    assert!(!temp.exists());
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/displayname")).unwrap(),
        "After"
    );
    fs::write(cfg.vdir.join("synthetic/held.ics"), "remote edit").unwrap();
    // Refuse changed task bytes before deleting even a recognized scratch file.
    fs::write(&temp, "Af").unwrap();
    assert!(operation::finalize_metadata(&cfg, &mut record).is_err());
    assert_eq!(fs::read_to_string(&temp).unwrap(), "Af");
}

#[test]
fn metadata_temp_recovery_still_refuses_unexpected_or_changed_bytes() {
    for (name, bytes) in [
        (".displayname.tdo-tmp", "unexpected"),
        (".color.tdo-tmp", "#379417extra"),
        (".held.ics.tdo-tmp", "held synthetic bytes"),
        ("held.ics", "changed task"),
    ] {
        let (_root, cfg, request) = setup();
        fs::write(cfg.vdir.join("synthetic/held.ics"), "held synthetic bytes").unwrap();
        let mut record = operation::prepare(&cfg, request, Utc::now()).unwrap();
        operation::set_phase(&cfg, &mut record, Phase::RemoteStarted).unwrap();
        record.receipt = Some(serde_json::json!({"name":"After", "color":"#379417"}));
        operation::set_phase(&cfg, &mut record, Phase::RemoteConfirmed).unwrap();
        operation::finalize_metadata(&cfg, &mut record).unwrap();
        let path = cfg.vdir.join("synthetic").join(name);
        fs::write(&path, bytes).unwrap();
        assert!(
            operation::finalize_metadata(&cfg, &mut record).is_err(),
            "{name}"
        );
        assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    }
}

#[test]
fn metadata_finalization_preserves_preexisting_temporary_files() {
    let (_root, cfg, request) = setup();
    let path = cfg.vdir.join("synthetic/.displayname.tdo-tmp");
    fs::write(&path, "A").unwrap();
    let mut record = operation::prepare(&cfg, request, Utc::now()).unwrap();
    operation::set_phase(&cfg, &mut record, Phase::RemoteStarted).unwrap();
    record.receipt = Some(serde_json::json!({"name":"After", "color":"#379417"}));
    operation::set_phase(&cfg, &mut record, Phase::RemoteConfirmed).unwrap();
    assert!(operation::finalize_metadata(&cfg, &mut record).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "A");
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/displayname")).unwrap(),
        "Synthetic"
    );
}
