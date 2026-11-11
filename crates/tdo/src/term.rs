//! Terminal loop: raw mode, key events, and the daemon's updates through the [`link`].
//!
//! The loop owns the [`App`]. The daemon owns the data: snapshots, sync status, refusals and
//! the answers to writes arrive as [`Update`]s from the link thread, and every action the user
//! takes that needs the daemon leaves as a [`Command`]. Nothing here reads the vdir, the
//! configuration or a lock file.

pub mod link;

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};
use twydo_core::config::Config;
use twydo_core::sync::resolve::Keep;
use twydo_core::views::Snapshot;
use twydo_daemon::protocol::admin::WireKeep;
use twydo_daemon::protocol::methods::{BasedOn, ConfigStatus};

use crate::app::{App, ClientSync, Outcome, SYNC_DONE};
use crate::ui;
use crate::write::Request;
use link::{Command, Link, NO_ANSWER, Update, WriteRequest};

/// How long the loop waits for a key before it looks at the daemon's updates again.
const POLL: Duration = Duration::from_millis(100);
const TICK_EVERY: Duration = Duration::from_secs(30);
/// How long a write waits for the daemon's answer (longer than the link's own 5 s call limit, so a slow fetch after a saved edit is not reported as a failure).
const WRITE_WAIT: Duration = Duration::from_secs(8);
/// The message of a TUI that has not heard from the daemon yet.
const CONNECTING: &str = "connecting to the daemon…";
/// Said when a binding setting of the configuration changed under a running daemon.
const RESTART: &str = "restart the daemon (tdo daemon stop) and reopen tdo";

/// What the loop does after a [`step`].
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
    /// Edit these notes in `$EDITOR`, then report back with [`App::notes_edited`].
    EditNotes(String),
}

/// The loop's side of the link: the link itself and what the loop remembers between steps.
pub struct Session {
    link: Link,
    /// The `seq` of the next write.
    seq: u64,
    /// The write the loop is waiting for, and its answer once it came.
    awaiting: Option<u64>,
    answer: Option<Result<String, String>>,
    /// The `Down` text on the status line, cleared when the link connects.
    down: Option<String>,
    /// Refusals already shown (and acknowledged) in this run.
    seen_refusals: HashSet<String>,
    /// The daemon's notes about its configuration and queue, as last shown.
    notes: Vec<String>,
    /// Whether the daemon's last status said a sync is running.
    running: bool,
    last_tick: Instant,
}

impl Session {
    /// A session over `link` for `app`. An app that has not seen a snapshot ([`first_app`]
    /// without a daemon) carries a message about the connection, which the first
    /// `Connected` clears.
    pub fn new(link: Link, app: &App) -> Session {
        Session {
            link,
            seq: 0,
            awaiting: None,
            answer: None,
            down: (app.snapshot_revision == 0)
                .then(|| app.message.clone())
                .flatten(),
            seen_refusals: HashSet::new(),
            notes: Vec::new(),
            running: false,
            last_tick: Instant::now(),
        }
    }

    pub fn link(&self) -> &Link {
        &self.link
    }
}

/// Warns once that the daemon will not start at login, unless the configuration turns the
/// warning off (`[daemon] unit_warning = false`). Without a first snapshot the switch is
/// unknown and the connection message stays.
pub fn unit_notice(app: &mut App, unit_installed: bool) {
    if !unit_installed && app.snapshot_revision != 0 && app.snapshot.config.daemon.unit_warning {
        app.notice("daemon won't start at login; run tdo daemon install");
    }
}

/// The app to start with: built from the first snapshot the link delivers within `wait`.
/// Without one (the daemon is down, or slow to start) the app starts empty with a message
/// about the connection; the loop fills it in when the link connects. Configuration warnings
/// are the first status-line message.
pub fn first_app(link: &Link, vdir: Option<&Path>, wait: Duration) -> App {
    let end = Instant::now() + wait;
    let mut down = None;
    let mut first = None;
    loop {
        match link.next_update(end.saturating_duration_since(Instant::now())) {
            Some(Update::Snapshot {
                revision, snapshot, ..
            }) => {
                first = Some((revision, snapshot));
                break;
            }
            Some(Update::Down(m)) => {
                down = Some(m);
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    let (revision, snapshot) = first.unwrap_or_else(|| {
        let vdir = vdir.map(Path::to_path_buf).unwrap_or_default();
        (0, Snapshot::empty(Config::with_vdir(vdir)))
    });
    let tz = snapshot.tz();
    let warnings = snapshot.config.warnings();
    let mut app = App::new(snapshot, Utc::now().with_timezone(&tz));
    app.snapshot_revision = revision;
    if revision == 0 {
        app.message = Some(down.unwrap_or_else(|| CONNECTING.into()));
    } else if !warnings.is_empty() {
        app.message = Some(warnings.join("; "));
    }
    app
}

/// Run the TUI until the user quits. The terminal is restored on exit and on panic.
pub fn run(mut app: App, link: Link, tz: Tz) -> io::Result<()> {
    let mut session = Session::new(link, &app);
    let mut terminal = init_terminal();
    let result = event_loop(&mut terminal, &mut app, &mut session, tz);
    restore_terminal();
    // Quitting starts no sync: the daemon decides when to sync.
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    session: &mut Session,
    tz: Tz,
) -> io::Result<()> {
    let size = terminal.size()?;
    app.resize(size.width, size.height);
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let event = if event::poll(POLL)? {
            Some(event::read()?)
        } else {
            None
        };
        match step(app, session, event, Utc::now().with_timezone(&tz)) {
            Flow::Continue => {}
            Flow::Quit => return Ok(()),
            Flow::EditNotes(notes) => {
                restore_terminal();
                let editor = crate::editor::editor_command(
                    std::env::var("VISUAL").ok().as_deref(),
                    std::env::var("EDITOR").ok().as_deref(),
                );
                let result = crate::editor::edit_text(&editor, &notes);
                *terminal = init_terminal();
                terminal.clear()?;
                let size = terminal.size()?;
                app.resize(size.width, size.height);
                app.notes_edited(result);
            }
        }
    }
}

/// One turn of the loop: apply the daemon's updates, then the event (a key, a paste, a resize),
/// then the clock. Never waits for the daemon, except for the answer to a write the event
/// asked for (up to [`WRITE_WAIT`]; the next key is not handled before it).
pub fn step(app: &mut App, session: &mut Session, event: Option<Event>, now: DateTime<Tz>) -> Flow {
    let updates: Vec<Update> = session.link.try_updates().collect();
    for update in updates {
        handle(app, session, update);
    }
    let mut flow = Flow::Continue;
    let mut ticked = false;
    match event {
        Some(Event::Key(k)) if k.kind != KeyEventKind::Release => {
            let o = app.key_at(k, now);
            flow = outcome(app, session, o);
            ticked = true;
        }
        Some(Event::Paste(text)) => {
            app.paste(&text);
            ticked = true;
        }
        Some(Event::Resize(w, h)) => {
            app.resize(w, h);
            ticked = true;
        }
        _ => {}
    }
    app.sync_activity.observe(session.running, Instant::now());
    app.expire_message(now);
    if ticked || session.last_tick.elapsed() >= TICK_EVERY {
        app.tick(now);
        session.last_tick = Instant::now();
    }
    flow
}

/// Sends what `o` asks of the daemon.
fn outcome(app: &mut App, session: &mut Session, o: Outcome) -> Flow {
    let link = &session.link;
    match o {
        Outcome::Continue => {}
        Outcome::Quit => return Flow::Quit,
        Outcome::EditNotes(notes) => return Flow::EditNotes(notes),
        // `sync.run` waits its turn behind a running job, so nothing is refused here.
        Outcome::Sync { force_delete } => link.send(Command::Sync {
            force_delete: force_delete.into_iter().collect(),
        }),
        Outcome::SyncSwitch(on) => link.send(Command::SetAuto(on)),
        Outcome::Resolve { id, keep } => link.send(Command::Resolve {
            id,
            keep: match keep {
                Keep::Local => WireKeep::Local,
                Keep::Remote => WireKeep::Remote,
                Keep::Current => WireKeep::Current,
            },
        }),
        Outcome::ForgetList(preview) => link.send(Command::Forget {
            name: preview.name,
            collection: preview.collection,
        }),
        Outcome::ManageList(action) => link.send(Command::List(action)),
        Outcome::Write(request) => write(app, session, request),
    }
    Flow::Continue
}

/// Sends a write and waits for its answer (see [`WRITE_WAIT`]), handling other updates meanwhile.
fn write(app: &mut App, session: &mut Session, request: Request) {
    let Some(held) = session.link.based_on() else {
        // The daemon never sent a snapshot: nothing here can be planned against it.
        app.write_refused();
        let why = session
            .down
            .clone()
            .unwrap_or_else(|| "daemon not running; try again".into());
        return app.write_finished(Err(why));
    };
    session.seq += 1;
    let seq = session.seq;
    session.awaiting = Some(seq);
    session.answer = None;
    session.link.send(Command::Write {
        seq,
        write: WriteRequest {
            request,
            based_on: BasedOn {
                epoch: held.epoch,
                revision: app.write_based_on(),
            },
        },
    });
    let end = Instant::now() + WRITE_WAIT;
    let result = loop {
        if let Some(r) = session.answer.take() {
            break r;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // The link still holds the write and sends it again after it reconnects; a late
            // answer is ignored, and the data arrives as a snapshot.
            break Err(NO_ANSWER.to_string());
        }
        if let Some(update) = session.link.next_update(left) {
            handle(app, session, update);
        }
    };
    session.awaiting = None;
    if result.is_err() {
        app.write_refused();
    }
    app.write_finished(result);
}

/// Applies one update from the link to the app.
pub fn handle(app: &mut App, session: &mut Session, update: Update) {
    match update {
        Update::Snapshot {
            revision, snapshot, ..
        } => {
            app.snapshot_revision = revision;
            app.reload(snapshot);
        }
        Update::Status(status) => status_update(app, session, status),
        Update::Down(message) => {
            session.down = Some(message.clone());
            app.message = Some(message);
        }
        Update::Connected { restarted, .. } => {
            if let Some(down) = session.down.take()
                && app.message.as_deref() == Some(down.as_str())
            {
                app.message = None;
            }
            if restarted {
                app.notice("daemon restarted");
            }
        }
        Update::Written { seq, result } => {
            if session.awaiting == Some(seq) {
                session.answer = Some(result);
            }
        }
        // A sync that went well says nothing; the status line shows the new state.
        Update::SyncDone(message) if message == SYNC_DONE => {}
        Update::SyncDone(message) => app.sync_finished(message),
        Update::Forgot(message) => app.message = Some(message),
        Update::ListDone(completion) => app.list_finished(completion),
    }
}

/// The daemon's status: the sync summary for the status line and the conflict overlay, the
/// refusals nobody has seen (shown once, then acknowledged), and what is wrong with its
/// configuration or queue (shown once per change).
fn status_update(app: &mut App, session: &mut Session, status: ClientSync) {
    session.running = status.running();
    let fresh: Vec<_> = status
        .refusals
        .iter()
        .filter(|r| session.seen_refusals.insert(r.id.clone()))
        .collect();
    let mut shown: Vec<String> = fresh.iter().map(|r| r.message.clone()).collect();
    if !fresh.is_empty() {
        session
            .link
            .send(Command::Ack(fresh.iter().map(|r| r.id.clone()).collect()));
    }
    let notes = notes_of(&status);
    if notes != session.notes {
        shown.extend(notes.iter().cloned());
        session.notes = notes;
    }
    if !shown.is_empty() {
        let shown = shown.join(" · ");
        app.message = Some(match app.message.take() {
            Some(m) if m != SYNC_DONE && !fresh.is_empty() => format!("{shown} · {m}"),
            _ => shown,
        });
    }
    let configured = status.sync.is_some();
    app.set_sync(configured.then_some(status));
}

/// What the status says is wrong, one line each.
fn notes_of(status: &ClientSync) -> Vec<String> {
    let ConfigStatus {
        error,
        restart_needed,
    } = &status.config;
    let mut notes = Vec::new();
    if let Some(e) = error {
        notes.push(format!("Cannot reload list preferences: {e}"));
    }
    if !restart_needed.is_empty() {
        notes.push(format!(
            "Configuration changed ({}); {RESTART} to use it",
            restart_needed.join(", ")
        ));
    }
    if let Some(e) = &status.drain_error {
        notes.push(format!("queued edits not saved: {e}"));
    }
    notes
}

/// Raw mode and the alternate screen, with bracketed paste so a paste arrives as one
/// event rather than as keys.
fn init_terminal() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = ratatui::crossterm::execute!(io::stdout(), EnableBracketedPaste);
    terminal
}

fn restore_terminal() {
    let _ = ratatui::crossterm::execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ConnectError;
    use twydo_core::views::View;
    use twydo_daemon::protocol::events::Cause;
    use twydo_daemon::protocol::methods::RefusalView;

    const FORCE_DELETE: &str = "list \"inbox\" was emptied on the server; to apply that here run: tdo sync --force-delete \"inbox\"";

    /// A link that never connects, so nothing but the test feeds the loop.
    fn offline() -> Link {
        Link::start_with(Box::new(|_| Err(ConnectError::Unavailable("test".into()))))
    }

    fn snapshot(titles: &[&str]) -> Snapshot {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("inbox");
        std::fs::create_dir_all(&list).unwrap();
        for (i, t) in titles.iter().enumerate() {
            std::fs::write(
                list.join(format!("t{i}.ics")),
                format!(
                    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-{i}\r\nSUMMARY:{t}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
                ),
            )
            .unwrap();
        }
        let cfg = Config::with_vdir(dir.path().to_path_buf());
        Snapshot::new(twydo_core::vdir::load(dir.path()).unwrap(), cfg)
    }

    fn update(revision: u64, titles: &[&str]) -> Update {
        Update::Snapshot {
            revision,
            snapshot: snapshot(titles),
            cause: Cause::Unknown,
            uids: Vec::new(),
        }
    }

    /// An app and session on a first snapshot with one task in `inbox`.
    fn inbox() -> (App, Session) {
        let first = snapshot(&["Synthetic task"]);
        let mut app = App::new(first, Utc::now().with_timezone(&chrono_tz::UTC));
        app.view = View::List("inbox".into());
        app.snapshot_revision = 1;
        assert_eq!(app.visible_rows().len(), 1);
        let session = Session::new(offline(), &app);
        (app, session)
    }

    fn later() -> DateTime<Tz> {
        Utc::now().with_timezone(&chrono_tz::UTC) + chrono::Duration::hours(1)
    }

    fn refusal(id: &str, message: &str) -> crate::app::ClientSync {
        let mut s = status();
        s.refusals.push(RefusalView {
            id: id.into(),
            message: message.into(),
            session: None,
            request_id: None,
            at: Utc::now(),
        });
        s
    }

    fn status() -> ClientSync {
        ClientSync {
            sync: None,
            unsynced: 0,
            queued: 0,
            conflicts: Vec::new(),
            refusals: Vec::new(),
            config: ConfigStatus::default(),
            drain_error: None,
        }
    }

    #[test]
    fn sync_completion_after_an_emptied_list_shows_no_rows_and_no_message() {
        let (mut app, mut session) = inbox();
        // The daemon applied an empty server list and sent the snapshot, then the sync ended.
        handle(&mut app, &mut session, update(2, &[]));
        handle(&mut app, &mut session, Update::SyncDone(SYNC_DONE.into()));

        assert!(app.visible_rows().is_empty());
        assert!(app.snapshot.tasks.is_empty());
        assert_eq!(app.snapshot_revision, 2);
        assert_eq!(app.view, View::List("inbox".into()));
        assert_eq!(app.message, None);
    }

    #[test]
    fn a_failed_sync_keeps_its_message_after_the_snapshot() {
        let (mut app, mut session) = inbox();
        handle(&mut app, &mut session, update(2, &["Synthetic task"]));
        handle(
            &mut app,
            &mut session,
            Update::SyncDone(FORCE_DELETE.into()),
        );

        assert_eq!(app.message.as_deref(), Some(FORCE_DELETE));
        app.expire_message(later());
        assert_eq!(app.message.as_deref(), Some(FORCE_DELETE), "errors stay");
    }

    #[test]
    fn a_good_sync_leaves_the_message_alone() {
        let (mut app, mut session) = inbox();
        app.message = Some("undone: Synthetic task".into());
        handle(&mut app, &mut session, Update::SyncDone(SYNC_DONE.into()));
        assert_eq!(app.message.as_deref(), Some("undone: Synthetic task"));
    }

    #[test]
    fn the_down_message_stays_until_the_link_connects() {
        let (mut app, mut session) = inbox();
        let down = "daemon not running: gone; retrying";
        handle(&mut app, &mut session, Update::Down(down.into()));
        assert_eq!(app.message.as_deref(), Some(down));
        app.expire_message(later());
        assert_eq!(app.message.as_deref(), Some(down));

        handle(
            &mut app,
            &mut session,
            Update::Connected {
                epoch: "e".into(),
                restarted: false,
            },
        );
        assert_eq!(app.message, None);

        // Another message is not the daemon's to clear.
        handle(&mut app, &mut session, Update::Down(down.into()));
        app.message = Some("something else".into());
        handle(
            &mut app,
            &mut session,
            Update::Connected {
                epoch: "e".into(),
                restarted: false,
            },
        );
        assert_eq!(app.message.as_deref(), Some("something else"));
    }

    #[test]
    fn a_restarted_daemon_is_announced_once_and_the_note_clears_itself() {
        let (mut app, mut session) = inbox();
        handle(
            &mut app,
            &mut session,
            Update::Connected {
                epoch: "e2".into(),
                restarted: true,
            },
        );
        assert_eq!(app.message.as_deref(), Some("daemon restarted"));
        app.expire_message(Utc::now().with_timezone(&chrono_tz::UTC));
        app.expire_message(later());
        assert_eq!(app.message, None);
    }

    #[test]
    fn an_app_that_started_without_the_daemon_drops_its_message_on_connect() {
        let link = offline();
        let app = first_app(&link, None, Duration::from_millis(50));
        assert_eq!(app.snapshot_revision, 0);
        let mut app = app;
        assert!(app.message.is_some(), "it says what it is waiting for");
        let mut session = Session::new(link, &app);
        handle(
            &mut app,
            &mut session,
            Update::Connected {
                epoch: "e".into(),
                restarted: false,
            },
        );
        assert_eq!(app.message, None);
    }

    #[test]
    fn refusals_show_once_and_never_again_in_this_run() {
        let (mut app, mut session) = inbox();
        let r = refusal(
            "op-1",
            "Not saved: \"Synthetic task\" changed on another device",
        );
        handle(&mut app, &mut session, Update::Status(r.clone()));
        assert_eq!(
            app.message.as_deref(),
            Some("Not saved: \"Synthetic task\" changed on another device")
        );
        app.message = None;
        handle(&mut app, &mut session, Update::Status(r.clone()));
        assert_eq!(app.message, None, "the same refusal is not shown again");

        // Two new ones are joined; the older one is not repeated.
        let mut two = refusal("op-1", "first");
        two.refusals.push(r.refusals[0].clone());
        two.refusals[1].id = "op-2".into();
        two.refusals[1].message = "second".into();
        two.refusals.push(r.refusals[0].clone());
        two.refusals[2].id = "op-3".into();
        two.refusals[2].message = "third".into();
        handle(&mut app, &mut session, Update::Status(two));
        assert_eq!(app.message.as_deref(), Some("second · third"));
    }

    #[test]
    fn refusals_and_config_notes_in_one_status_are_both_shown() {
        let (mut app, mut session) = inbox();
        let mut s = refusal(
            "op-1",
            "Not saved: \"Synthetic task\" changed on another device",
        );
        s.config.restart_needed = vec!["vdir".into()];
        handle(&mut app, &mut session, Update::Status(s));
        let shown = app.message.unwrap();
        assert!(shown.starts_with("Not saved: "), "{shown}");
        assert!(shown.contains(" · Configuration changed (vdir)"), "{shown}");
    }

    #[test]
    fn a_configuration_problem_is_shown_once_per_change() {
        let (mut app, mut session) = inbox();
        let mut s = status();
        s.config.restart_needed = vec!["vdir".into()];
        handle(&mut app, &mut session, Update::Status(s.clone()));
        let shown = app.message.clone().unwrap();
        assert!(
            shown.contains("restart the daemon (tdo daemon stop) and reopen tdo"),
            "{shown}"
        );
        assert!(shown.contains("vdir"), "{shown}");

        app.message = None;
        handle(&mut app, &mut session, Update::Status(s.clone()));
        assert_eq!(app.message, None, "unchanged: not shown again");

        s.config.error = Some("bad".into());
        handle(&mut app, &mut session, Update::Status(s));
        assert!(
            app.message
                .unwrap()
                .contains("Cannot reload list preferences: bad")
        );
    }

    #[test]
    fn a_stale_answer_is_ignored_and_the_status_drives_the_sync_state() {
        let (mut app, mut session) = inbox();
        handle(
            &mut app,
            &mut session,
            Update::Written {
                seq: 9,
                result: Ok("late".into()),
            },
        );
        assert_eq!(app.message, None);
        assert!(session.answer.is_none());

        // Without `[sync]` the app keeps no sync state; with it, the status is kept.
        handle(&mut app, &mut session, Update::Status(status()));
        assert!(app.sync.is_none());
    }

    #[test]
    fn a_write_refused_at_once_while_down_leaves_the_card_with_the_reason() {
        let (mut app, mut session) = inbox();
        let down = "daemon not running: gone; retrying";
        handle(&mut app, &mut session, Update::Down(down.into()));
        // No snapshot was ever sent through this link, so nothing is planned against one.
        write(
            &mut app,
            &mut session,
            Request::Apply(twydo_core::edit::Op::Complete {
                uid: "synthetic-0".into(),
            }),
        );
        assert_eq!(app.message.as_deref(), Some(down));
    }
}
