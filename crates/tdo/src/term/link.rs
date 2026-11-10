//! The TUI's link to the daemon: one thread that owns the connection. The loop sends it
//! [`Command`]s and receives [`Update`]s, so nothing on the UI thread blocks on a socket.
//!
//! The thread connects (starting the daemon on demand), subscribes to events and fetches the
//! snapshot and status. It re-fetches the snapshot after `data.changed` (unless it already
//! holds that revision), after `missed` and after its own writes, and the status and
//! conflict list after `status.changed`. After a lost connection it reconnects with backoff
//! (250 ms doubling to 2 s), resuming its session, and resends the write that was in flight
//! with the same request id. Long jobs (a sync, list actions) never block the thread: they
//! are started, remembered, and reported when `job.finished` arrives (or `job.get` says so;
//! it is asked every 5 s and after a reconnect).

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryIter, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use twydo_core::lists::{ListRequest, ListResult, ListState};
use twydo_core::views::Snapshot;
use twydo_daemon::client::{Caller, Client, ClientError, Notification};
use twydo_daemon::protocol::admin::{
    ConflictList, ConflictsResolveParams, ResolveResult, WireKeep,
};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::jobs::{JobGetParams, JobResult, JobStarted, JobState, JobView};
use twydo_daemon::protocol::lists::{
    ListArchiveParams, ListCreateParams, ListDeleteParams, ListExportParams, ListForgetParams,
    ListPreviewDeleteParams, ListReconcileParams, ListRecoverParams, ListUpdateParams,
    PendingListOp, WireListState, color_hex,
};
use twydo_daemon::protocol::methods::{
    AckParams, BasedOn, HelloResult, StatusView, SubmitParams, SubscribeResult, UndoParams,
    WriteResult,
};
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;
use twydo_daemon::protocol::sync::{
    SyncOutcome, SyncResult, SyncRunParams, SyncSetAutoParams, SyncSetAutoResult,
};
use twydo_daemon::protocol::wire::{WireSnapshot, from_wire};

use crate::app::{ClientSync, ListAction, ListCompletion, SYNC_DONE};
use crate::client::{self, ConnectError, Target};
use crate::write::Request;

const CLIENT: &str = "tdo-tui";
/// How long a call from the link thread may take.
const CALL: Duration = Duration::from_secs(5);
/// How long a call waits for a daemon that is still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(30);
/// How often an unfinished job is asked about when no event arrives.
const JOB_POLL: Duration = Duration::from_secs(5);
/// How often the thread looks at the command queue and the events.
const TICK: Duration = Duration::from_millis(20);
const BACKOFF_FIRST: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

/// The text of a call that timed out.
pub const NO_ANSWER: &str = "daemon did not answer; try again";

/// A write the loop asks for: the request and the snapshot revision it was planned against.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteRequest {
    pub request: Request,
    pub based_on: BasedOn,
}

// Messages are sent one at a time; boxing the large variants would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Command {
    /// A write; `seq` is chosen by the sender and echoed by [`Update::Written`], so a late
    /// reply is never taken for the answer to a newer write.
    Write {
        seq: u64,
        write: WriteRequest,
    },
    Sync {
        force_delete: Vec<String>,
    },
    SetAuto(bool),
    Resolve {
        id: String,
        keep: WireKeep,
    },
    Forget {
        name: String,
        collection: String,
    },
    List(ListAction),
    /// Acknowledge refusals that were shown (`refusals.ack`).
    Ack(Vec<String>),
    Quit,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Update {
    /// Connected (again); `restarted` when the daemon is not the one seen before.
    Connected {
        epoch: String,
        restarted: bool,
    },
    /// Not connected: `daemon not running: ...; retrying`.
    Down(String),
    Snapshot {
        revision: u64,
        snapshot: Snapshot,
        cause: Cause,
        uids: Vec<String>,
    },
    Status(ClientSync),
    /// The reply to [`Command::Write`]: the message, or why nothing was written.
    Written {
        seq: u64,
        result: Result<String, String>,
    },
    /// A sync ended ([`sync_message`]), or the answer to `SetAuto` or `Resolve`.
    SyncDone(String),
    Forgot(String),
    ListDone(ListCompletion),
}

/// How the link connects: `None` for no session yet, else the one to resume.
pub type Connector =
    Box<dyn FnMut(Option<String>) -> Result<(Client, HelloResult), ConnectError> + Send>;

pub struct Link {
    tx: Sender<Command>,
    rx: Receiver<Update>,
    caller: Arc<Mutex<Option<Caller>>>,
    based_on: Arc<Mutex<Option<BasedOn>>>,
}

impl Link {
    /// Starts the thread; it connects on its own (starting the daemon when none runs).
    pub fn start(target: Target) -> Link {
        Link::start_with(Box::new(move |session| {
            client::connect(&target, CLIENT, session)
        }))
    }

    /// [`Link::start`] with another way to connect (tests).
    pub fn start_with(connect: Connector) -> Link {
        let (tx, cmd_rx) = mpsc::channel();
        let (up_tx, rx) = mpsc::channel();
        let caller = Arc::new(Mutex::new(None));
        let based_on = Arc::new(Mutex::new(None));
        let thread = Thread {
            connect,
            cmds: cmd_rx,
            ups: up_tx,
            caller: Arc::clone(&caller),
            based_on: Arc::clone(&based_on),
            session: None,
            epoch: None,
            inflight: None,
            acks: Vec::new(),
            jobs: HashMap::new(),
            down: None,
        };
        std::thread::Builder::new()
            .name("tdo-link".into())
            .spawn(move || thread.run())
            .expect("spawn the link thread");
        Link {
            tx,
            rx,
            caller,
            based_on,
        }
    }

    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }

    pub fn try_updates(&self) -> TryIter<'_, Update> {
        self.rx.try_iter()
    }

    /// The updates, waiting up to `limit` for the next one (tests).
    pub fn next_update(&self, limit: Duration) -> Option<Update> {
        self.rx.recv_timeout(limit).ok()
    }

    /// A handle on the live connection, for blocking reads made on another thread (the
    /// panels); `None` while disconnected.
    pub fn caller(&self) -> Option<Caller> {
        self.caller.lock().unwrap().clone()
    }

    /// The epoch and revision of the last snapshot the link sent.
    pub fn based_on(&self) -> Option<BasedOn> {
        self.based_on.lock().unwrap().clone()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
    }
}

/// The status-line text of a finished sync, as `tdo sync` words it: problems, then warnings
/// as `warning: <w>`, joined with ` · `; [`SYNC_DONE`] when there is nothing to say and the
/// run was fine (or did nothing), `sync failed` or `sync stopped` otherwise.
pub fn sync_message(r: &SyncResult) -> String {
    let mut parts: Vec<String> = r.problems.clone();
    parts.extend(r.warnings.iter().map(|w| format!("warning: {w}")));
    if !parts.is_empty() {
        return parts.join(" · ");
    }
    match r.outcome {
        SyncOutcome::Ok | SyncOutcome::Skipped => SYNC_DONE.into(),
        SyncOutcome::Failed => "sync failed".into(),
        SyncOutcome::Stopped | SyncOutcome::Unknown => "sync stopped".into(),
    }
}

/// A write that was sent and not yet answered.
struct Inflight {
    seq: u64,
    request_id: String,
    write: WriteRequest,
}

/// What a started job will report.
enum Job {
    Sync,
    List(ListAction),
    Forget,
    /// A recovery, answered as the write `seq`.
    Recover(u64),
}

/// Why the connection is over.
enum Lost {
    Quit,
    /// The connection failed (or timed out); the message says why.
    Failed(String),
}

struct Thread {
    connect: Connector,
    cmds: Receiver<Command>,
    ups: Sender<Update>,
    caller: Arc<Mutex<Option<Caller>>>,
    based_on: Arc<Mutex<Option<BasedOn>>>,
    session: Option<String>,
    epoch: Option<String>,
    inflight: Option<Inflight>,
    jobs: HashMap<String, Job>,
    /// Refusals the loop acknowledged while down; sent after the next connect.
    acks: Vec<String>,
    /// The last `Down` text, so an outage is reported once.
    down: Option<String>,
}

/// What the connection has fetched so far.
struct Held {
    revision: u64,
    status_rev: u64,
    epoch: String,
}

impl Thread {
    fn run(mut self) {
        let mut backoff = BACKOFF_FIRST;
        loop {
            match (self.connect)(self.session.clone()) {
                Ok((client, hello)) => {
                    backoff = BACKOFF_FIRST;
                    self.session = Some(hello.session.clone());
                    let restarted = self.epoch.as_ref().is_some_and(|e| *e != hello.epoch);
                    self.epoch = Some(hello.epoch.clone());
                    self.down = None;
                    *self.caller.lock().unwrap() = Some(client.caller());
                    self.send(Update::Connected {
                        epoch: hello.epoch.clone(),
                        restarted,
                    });
                    let lost = self.serve(&client, &hello);
                    *self.caller.lock().unwrap() = None;
                    drop(client);
                    match lost {
                        Lost::Quit => return,
                        Lost::Failed(why) => {
                            self.report_down(format!("daemon not running: {why}; retrying"))
                        }
                    }
                }
                Err(e) => self.report_down(format!("{e}; retrying")),
            }
            if !self.idle(backoff) {
                return;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    fn send(&self, u: Update) {
        let _ = self.ups.send(u);
    }

    fn report_down(&mut self, text: String) {
        if self.down.as_deref() != Some(text.as_str()) {
            self.down = Some(text.clone());
            self.send(Update::Down(text));
        }
    }

    /// Waits `limit` while disconnected, answering what the loop sends so nothing waits for
    /// a daemon that is away. False once told to quit.
    fn idle(&mut self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            loop {
                match self.cmds.try_recv() {
                    Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return false,
                    Ok(c) => self.refuse_while_down(c),
                    Err(TryRecvError::Empty) => break,
                }
            }
            std::thread::sleep(TICK);
        }
        true
    }

    fn refuse_while_down(&mut self, c: Command) {
        let why = self
            .down
            .clone()
            .unwrap_or_else(|| "daemon not running; retrying".into());
        match c {
            // The write in flight is kept for the next connection; a new one is not taken.
            Command::Write { seq, .. } => self.send(Update::Written {
                seq,
                result: Err(if self.inflight.is_some() {
                    "a previous save is still being sent; try again".into()
                } else {
                    why
                }),
            }),
            Command::Sync { .. } | Command::SetAuto(_) | Command::Resolve { .. } => {
                self.send(Update::SyncDone(why))
            }
            Command::Forget { .. } => {
                self.send(Update::Forgot(format!("list was not removed: {why}")))
            }
            Command::List(action) => self.send(Update::ListDone(ListCompletion {
                action,
                result: Err(why),
                pending: None,
                server_values: None,
            })),
            Command::Ack(ids) => self.acks.extend(ids),
            Command::Quit => {}
        }
    }

    /// One connection's life.
    fn serve(&mut self, client: &Client, hello: &HelloResult) -> Lost {
        let caller = client.caller();
        let mut held = Held {
            revision: 0,
            status_rev: 0,
            epoch: hello.epoch.clone(),
        };
        if let Err(e) = self.start_up(&caller, &mut held) {
            return Lost::Failed(e);
        }
        if let Err(e) = self.send_acks(&caller) {
            return Lost::Failed(e);
        }
        if let Some(i) = self.inflight.take() {
            // The connection dropped (or timed out) under this write: ask again with the
            // same request id; the daemon replays an answer it already gave.
            if let Err(lost) = self.write(client, &mut held, i) {
                return lost;
            }
        }
        let mut last_poll = Instant::now();
        loop {
            loop {
                match self.cmds.try_recv() {
                    Ok(Command::Quit) | Err(TryRecvError::Disconnected) => return Lost::Quit,
                    Ok(c) => {
                        if let Err(lost) = self.command(client, &mut held, c) {
                            return lost;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            match client.notifications().recv_timeout(TICK) {
                Ok(n) => {
                    if let Err(e) = self.notification(&caller, &mut held, n) {
                        return Lost::Failed(e);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Lost::Failed("the daemon closed the connection".into());
                }
            }
            if !self.jobs.is_empty() && last_poll.elapsed() >= JOB_POLL {
                last_poll = Instant::now();
                if let Err(e) = self.poll_jobs(&caller) {
                    return Lost::Failed(e);
                }
            }
        }
    }

    /// Subscribes, then fetches everything and checks the jobs still outstanding.
    fn start_up(&mut self, caller: &Caller, held: &mut Held) -> Result<(), String> {
        let _: SubscribeResult =
            ready(caller, "events.subscribe", &serde_json::json!({})).map_err(|e| e.to_string())?;
        self.fetch_snapshot(caller, held, Cause::Unknown, Vec::new())
            .map_err(|e| e.to_string())?;
        self.fetch_status(caller, held).map_err(|e| e.to_string())?;
        self.poll_jobs(caller)
    }

    fn fetch_snapshot(
        &self,
        caller: &Caller,
        held: &mut Held,
        cause: Cause,
        uids: Vec<String>,
    ) -> Result<(), ClientError> {
        let wire: WireSnapshot = ready(caller, "snapshot.get", &serde_json::json!({}))?;
        held.revision = wire.revision;
        let revision = wire.revision;
        *self.based_on.lock().unwrap() = Some(BasedOn {
            epoch: held.epoch.clone(),
            revision,
        });
        self.send(Update::Snapshot {
            revision,
            snapshot: from_wire(wire),
            cause,
            uids,
        });
        Ok(())
    }

    fn fetch_status(&self, caller: &Caller, held: &mut Held) -> Result<(), ClientError> {
        let status: StatusView = ready(caller, "status.get", &serde_json::json!({}))?;
        let list: ConflictList = ready(caller, "conflicts.list", &serde_json::json!({}))?;
        held.status_rev = status.status_rev;
        self.send(Update::Status(ClientSync::from_status(
            &status,
            list.conflicts,
        )));
        Ok(())
    }

    fn notification(
        &mut self,
        caller: &Caller,
        held: &mut Held,
        n: Notification,
    ) -> Result<(), String> {
        let broken = |e: ClientError| e.to_string();
        match n {
            Notification::Heartbeat(_) => {}
            Notification::Missed(_) => {
                self.fetch_snapshot(caller, held, Cause::Unknown, Vec::new())
                    .map_err(broken)?;
                self.fetch_status(caller, held).map_err(broken)?;
                self.poll_jobs(caller)?;
            }
            Notification::Event(env) => match env.event {
                Event::DataChanged {
                    revision,
                    cause,
                    uids,
                } if revision > held.revision => {
                    self.fetch_snapshot(caller, held, cause, uids)
                        .map_err(broken)?;
                }
                Event::StatusChanged { status_rev } if status_rev > held.status_rev => {
                    self.fetch_status(caller, held).map_err(broken)?;
                }
                Event::JobFinished { job_id, state, .. } => {
                    self.job_ended(caller, &job_id, state);
                }
                _ => {}
            },
        }
        Ok(())
    }

    fn command(&mut self, client: &Client, held: &mut Held, c: Command) -> Result<(), Lost> {
        let caller = &client.caller();
        match c {
            Command::Quit => Err(Lost::Quit),
            Command::Write { seq, write } => {
                let request_id = caller.next_request_id();
                self.write(
                    client,
                    held,
                    Inflight {
                        seq,
                        request_id,
                        write,
                    },
                )
            }
            Command::Sync { force_delete } => {
                let id = caller.next_request_id();
                let started = call::<_, JobStarted>(
                    caller,
                    "sync.run",
                    &SyncRunParams {
                        request_id: format!("sync-{id}"),
                        force_delete,
                    },
                );
                self.started(caller, started, Job::Sync, Update::SyncDone)
            }
            Command::SetAuto(on) => {
                let params = SyncSetAutoParams {
                    request_id: format!("sync-{}", caller.next_request_id()),
                    on,
                };
                match call::<_, SyncSetAutoResult>(caller, "sync.set_auto", &params) {
                    Ok(r) => self.send(Update::SyncDone(r.message)),
                    Err(e) => self.answer_failed(e, Update::SyncDone)?,
                }
                Ok(())
            }
            Command::Resolve { id, keep } => {
                let params = ConflictsResolveParams {
                    request_id: format!("resolve-{}", caller.next_request_id()),
                    id,
                    keep,
                };
                match call::<_, ResolveResult>(caller, "conflicts.resolve", &params) {
                    Ok(r) => self.send(Update::SyncDone(r.message)),
                    Err(e) => {
                        self.answer_failed(e, |m| Update::SyncDone(format!("resolve failed: {m}")))?
                    }
                }
                Ok(())
            }
            Command::Forget { name, collection } => {
                let params = ListForgetParams {
                    request_id: format!("forget-{}", caller.next_request_id()),
                    name,
                    collection,
                };
                let started = call::<_, JobStarted>(caller, "lists.forget", &params);
                self.started(caller, started, Job::Forget, |m| {
                    Update::Forgot(format!("list was not removed: {m}"))
                })
            }
            Command::List(action) => {
                let rid = format!("lists-{}", caller.next_request_id());
                let (method, params) = list_call(&action, rid);
                let started = call::<_, JobStarted>(caller, method, &params);
                let failed = action.clone();
                self.started(caller, started, Job::List(action), move |m| {
                    Update::ListDone(ListCompletion {
                        action: failed.clone(),
                        result: Err(m),
                        pending: None,
                        server_values: None,
                    })
                })
            }
            Command::Ack(ids) => {
                let params = AckParams {
                    request_id: caller.next_request_id(),
                    ids,
                };
                match call::<_, serde_json::Value>(caller, "refusals.ack", &params) {
                    Err(e) if is_connection(&e) => {
                        self.acks.extend(params.ids);
                        Err(Lost::Failed(e.to_string()))
                    }
                    _ => Ok(()),
                }
            }
        }
    }

    /// A call that failed on an answer: an error the daemon gave is the reply; a lost
    /// connection ends this one.
    fn answer_failed(
        &self,
        e: ClientError,
        reply: impl FnOnce(String) -> Update,
    ) -> Result<(), Lost> {
        match e {
            ClientError::Rpc(e) => {
                self.send(reply(e.message));
                Ok(())
            }
            // The daemon's reply made no sense: an answer, not a lost connection.
            ClientError::Decode(m) => {
                self.send(reply(m));
                Ok(())
            }
            ClientError::TimedOut(_) => {
                self.send(reply(NO_ANSWER.into()));
                Err(Lost::Failed(NO_ANSWER.into()))
            }
            e => {
                self.send(reply("daemon connection lost; try again".into()));
                Err(Lost::Failed(e.to_string()))
            }
        }
    }

    /// Remembers the job a call started, or reports why it did not; checks it once at
    /// once, in case its event came first.
    fn started(
        &mut self,
        caller: &Caller,
        started: Result<JobStarted, ClientError>,
        job: Job,
        failed: impl FnOnce(String) -> Update,
    ) -> Result<(), Lost> {
        match started {
            Ok(s) => {
                self.jobs.insert(s.job_id.clone(), job);
                self.check_job(caller, &s.job_id).map_err(Lost::Failed)
            }
            Err(e) => self.answer_failed(e, failed),
        }
    }

    /// Sends a write and reports the answer, tied to the write by `seq`. Whenever no answer
    /// arrived (the connection dropped, or the call timed out and the link reconnects) the
    /// write stays in flight and is sent again, with the same request id, after the next
    /// connect.
    fn write(&mut self, client: &Client, held: &mut Held, i: Inflight) -> Result<(), Lost> {
        let caller = &client.caller();
        let seq = i.seq;
        let session = caller.session().unwrap_or_default();
        if let Request::Recover { backup, to } = &i.write.request {
            let params = ListRecoverParams {
                session,
                request_id: i.request_id.clone(),
                backup: backup.clone(),
                to: to.clone(),
            };
            return match call::<_, JobStarted>(caller, "lists.recover", &params) {
                Ok(s) => {
                    self.jobs.insert(s.job_id.clone(), Job::Recover(seq));
                    self.check_job(caller, &s.job_id).map_err(Lost::Failed)
                }
                Err(e) => self.write_failed(i, e),
            };
        }
        let sent: Result<WriteResult, ClientError> = match &i.write.request {
            Request::Apply(op) => call(
                caller,
                "op.submit",
                &SubmitParams {
                    session,
                    request_id: i.request_id.clone(),
                    based_on: i.write.based_on.clone(),
                    op: op.clone(),
                },
            ),
            Request::Undo | Request::Redo => call(
                caller,
                if i.write.request == Request::Undo {
                    "op.undo"
                } else {
                    "op.redo"
                },
                &UndoParams {
                    session,
                    request_id: i.request_id.clone(),
                },
            ),
            Request::Recover { .. } => unreachable!("handled above"),
        };
        match sent {
            Ok(r) => {
                self.send(Update::Written {
                    seq,
                    result: Ok(r.message.clone()),
                });
                // The daemon publishes the change (`data.changed`, cause `op`, with the uids)
                // before it answers, so the event is already waiting: take it first, and it
                // brings the snapshot with its cause and uids. When it did not come (or not
                // yet), fetch the snapshot here as `Cause::Op` with no uids, so an own write
                // never looks like a change from outside; the later event for the same
                // revision is then skipped as a duplicate.
                self.take_waiting(client, held).map_err(Lost::Failed)?;
                if r.revision > held.revision {
                    self.fetch_snapshot(caller, held, Cause::Op, Vec::new())
                        .map_err(|e| Lost::Failed(e.to_string()))?;
                }
                Ok(())
            }
            Err(e) => self.write_failed(i, e),
        }
    }

    /// A write call that failed: an answer of the daemon (an error, or a reply that does not
    /// parse) is reported; anything else (a lost connection, a timeout) keeps the write in
    /// flight and ends this connection.
    fn write_failed(&mut self, i: Inflight, e: ClientError) -> Result<(), Lost> {
        match e {
            ClientError::Rpc(e) => {
                self.send(Update::Written {
                    seq: i.seq,
                    result: Err(e.message),
                });
                Ok(())
            }
            ClientError::Decode(m) => {
                self.send(Update::Written {
                    seq: i.seq,
                    result: Err(m),
                });
                Ok(())
            }
            e => {
                self.inflight = Some(i);
                Err(Lost::Failed(match e {
                    ClientError::TimedOut(_) => NO_ANSWER.to_string(),
                    e => e.to_string(),
                }))
            }
        }
    }

    /// Handles the notifications that are already waiting.
    fn take_waiting(&mut self, client: &Client, held: &mut Held) -> Result<(), String> {
        while let Ok(n) = client.notifications().try_recv() {
            self.notification(&client.caller(), held, n)?;
        }
        Ok(())
    }

    /// Sends the acknowledgements kept while down.
    fn send_acks(&mut self, caller: &Caller) -> Result<(), String> {
        if self.acks.is_empty() {
            return Ok(());
        }
        let params = AckParams {
            request_id: caller.next_request_id(),
            ids: std::mem::take(&mut self.acks),
        };
        match call::<_, serde_json::Value>(caller, "refusals.ack", &params) {
            Err(e) if is_connection(&e) => {
                self.acks = params.ids;
                Err(e.to_string())
            }
            _ => Ok(()),
        }
    }

    fn poll_jobs(&mut self, caller: &Caller) -> Result<(), String> {
        let ids: Vec<String> = self.jobs.keys().cloned().collect();
        for id in ids {
            self.check_job(caller, &id)?;
        }
        Ok(())
    }

    /// Asks `job.get`; an answer from the daemon that is not a job (it forgot it) ends the
    /// job as lost.
    fn check_job(&mut self, caller: &Caller, id: &str) -> Result<(), String> {
        let got: Result<JobView, ClientError> = call(
            caller,
            "job.get",
            &JobGetParams {
                job_id: Some(id.to_string()),
                request_id: None,
            },
        );
        match got {
            Ok(v) if v.state.is_final() => self.job_ended(caller, id, v.state),
            Ok(_) => {}
            Err(ClientError::Rpc(e))
                if e.data.as_ref().map(|d| d.kind) == Some(RpcKind::Unavailable) => {}
            Err(ClientError::Rpc(e)) => {
                self.job_ended(
                    caller,
                    id,
                    JobState::Failed {
                        kind: e.data.map_or(RpcKind::Internal, |d| d.kind),
                        message: e.message,
                    },
                );
            }
            Err(e) => return Err(e.to_string()),
        }
        Ok(())
    }

    /// Reports the end of job `id`, once.
    fn job_ended(&mut self, caller: &Caller, id: &str, state: JobState) {
        let Some(job) = self.jobs.remove(id) else {
            return;
        };
        let problem = |state: &JobState| match state {
            JobState::Failed { message, .. } => message.clone(),
            _ => "the daemon stopped before the job ended".into(),
        };
        match job {
            Job::Sync => self.send(Update::SyncDone(match state {
                JobState::Finished {
                    result: JobResult::Sync(r),
                } => sync_message(&r),
                other => problem(&other),
            })),
            Job::Forget => self.send(Update::Forgot(match state {
                JobState::Finished {
                    result: JobResult::Forgot { message, .. },
                } => message,
                other => format!("list was not removed: {}", problem(&other)),
            })),
            Job::Recover(seq) => self.send(Update::Written {
                seq,
                result: match state {
                    JobState::Finished {
                        result: JobResult::Recovered { message, .. },
                    } => Ok(message),
                    other => Err(problem(&other)),
                },
            }),
            Job::List(action) => {
                let result = list_result(state);
                let (pending, server_values) = pending_list(caller);
                self.send(Update::ListDone(ListCompletion {
                    action,
                    result,
                    pending,
                    server_values,
                }));
            }
        }
    }
}

/// The result a list job's final state stands for.
fn list_result(state: JobState) -> Result<ListResult, String> {
    match state {
        JobState::Finished { result } => match result {
            JobResult::ListSaved { operation, state } => Ok(ListResult::Saved {
                operation,
                state: ListState::try_from(state)?,
            }),
            JobResult::DeletePreview(p) => Ok(ListResult::DeletePreview(p.into())),
            JobResult::Exported {
                backup,
                destination,
            } => Ok(ListResult::Exported {
                backup,
                destination,
            }),
            _ => Err("the daemon answered with something unexpected".into()),
        },
        JobState::Failed { message, .. } => Err(message),
        _ => Err("the daemon stopped before the job ended".into()),
    }
}

/// The unfinished list operation, if any: `(operation, collection)` and the server's name
/// and colour when it reported them.
#[allow(clippy::type_complexity)]
fn pending_list(
    caller: &Caller,
) -> (
    Option<(String, String)>,
    Option<(Option<String>, Option<String>)>,
) {
    let got: Result<Option<PendingListOp>, _> =
        call(caller, "lists.pending", &serde_json::json!({}));
    let Ok(Some(p)) = got else {
        return (None, None);
    };
    let server = (p.observed_name.is_some() || p.observed_color.is_some())
        .then(|| (p.observed_name.clone(), p.observed_color.clone()));
    let pending = p.collection.map(|c| (p.operation, c));
    (pending, server)
}

/// The method and params that start the job `action` stands for.
fn list_call(action: &ListAction, request_id: String) -> (&'static str, serde_json::Value) {
    fn v<T: Serialize>(p: T) -> serde_json::Value {
        serde_json::to_value(p).expect("params serialize")
    }
    match action {
        ListAction::Save(ListRequest::Create { id, name, color }) => (
            "lists.create",
            v(ListCreateParams {
                request_id,
                id: id.clone(),
                name: name.clone(),
                color: color_hex(*color),
            }),
        ),
        ListAction::Save(ListRequest::Edit {
            id,
            name,
            color,
            expected,
        }) => (
            "lists.update",
            v(ListUpdateParams {
                request_id,
                id: id.clone(),
                name: name.clone(),
                color: color.map(color_hex),
                expected: WireListState::from(expected.clone()),
            }),
        ),
        ListAction::Archive { expected, archived } => (
            if *archived {
                "lists.archive"
            } else {
                "lists.unarchive"
            },
            v(ListArchiveParams {
                request_id,
                expected: WireListState::from(expected.clone()),
            }),
        ),
        ListAction::PreviewDelete { id } => (
            "lists.preview_delete",
            v(ListPreviewDeleteParams {
                request_id,
                id: id.clone(),
            }),
        ),
        ListAction::Delete(preview) => (
            "lists.delete",
            v(ListDeleteParams {
                request_id,
                preview: preview.clone().into(),
            }),
        ),
        ListAction::Reconcile {
            operation,
            keep_server,
        } => (
            "lists.reconcile",
            v(ListReconcileParams {
                request_id,
                operation: operation.clone(),
                keep_server: *keep_server,
            }),
        ),
        ListAction::ExportBackup {
            backup,
            destination,
        } => (
            "lists.export",
            v(ListExportParams {
                request_id,
                backup: backup.clone(),
                destination: destination.clone(),
            }),
        ),
    }
}

fn call<P: Serialize, R: serde::de::DeserializeOwned>(
    caller: &Caller,
    method: &str,
    params: &P,
) -> Result<R, ClientError> {
    caller.call_timeout(method, params, CALL)
}

/// [`call`], retried while the daemon says it is still starting.
fn ready<P: Serialize, R: serde::de::DeserializeOwned>(
    caller: &Caller,
    method: &str,
    params: &P,
) -> Result<R, ClientError> {
    client::call_ready(caller, method, params, CALL, STARTUP_WAIT)
}

/// Did the connection fail (rather than the daemon answer with an error)?
fn is_connection(e: &ClientError) -> bool {
    !matches!(e, ClientError::Rpc(_) | ClientError::Decode(_))
}
