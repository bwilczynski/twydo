//! Terminal loop: raw mode, key events, live reload of the vdir and the minute tick.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use chrono::Utc;
use chrono_tz::Tz;
use notify::event::{EventKind, MetadataKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};
use twydo_core::config::Config;
use twydo_core::store::queue;
use twydo_core::sync::batch::after_resolve;
use twydo_core::sync::resolve::{Resolution, resolve};

use crate::app::{App, Outcome};
use crate::ui;
use crate::write::Writer;

const DEBOUNCE: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(250);
const TICK_EVERY: Duration = Duration::from_secs(30);

/// Watch `root` recursively. One `()` arrives per change to the vdir. Reads are not changes:
/// every reload reads all files, so counting them would reload in a loop. The watcher must be
/// kept alive for as long as signals are wanted; the event loop debounces the signals.
pub fn watch(root: &Path) -> notify::Result<(RecommendedWatcher, Receiver<()>)> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let changed = match res {
            Ok(ev) => is_change(&ev.kind),
            Err(_) => false,
        };
        if changed {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

/// Creates, writes, renames and removes count; opens, reads and access-time updates do not.
fn is_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}

/// Run the TUI until the user quits. The terminal is restored on exit and on panic.
pub fn run(mut app: App, root: PathBuf, tz: Tz) -> io::Result<()> {
    let mut writer = Writer::open(&app.snapshot.config);
    let mut terminal = init_terminal();
    let result = event_loop(&mut terminal, &mut app, &mut writer, &root, tz);
    restore_terminal();
    // Quitting starts no sync: the batch waiter started by the last write outlives the TUI.
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    writer: &mut Writer,
    root: &Path,
    tz: Tz,
) -> io::Result<()> {
    // Configuration warnings are the first status-line message.
    let warnings = app.snapshot.config.warnings();
    if !warnings.is_empty() {
        app.message = Some(warnings.join("; "));
    }
    // Without a watcher the app still works; it just does not live-reload.
    let watcher = match watch(root) {
        Ok(w) => Some(w),
        Err(e) => {
            app.message
                .get_or_insert_with(|| format!("live reload unavailable: {e}"));
            None
        }
    };
    let (sync_tx, sync_rx) = mpsc::channel::<String>();
    let mut syncing = false;
    // Edits queued in an earlier session are written before the background sync starts (it
    // would hold the vdir), and refusals nobody has seen yet are shown.
    drain_and_reload(app, writer, root);
    // One background sync at start, then the first summary.
    crate::cli::sync::spawn_background(&app.snapshot.config, Some(&app.snapshot.config.vdir));
    refresh_sync(app);
    let now = || Utc::now().with_timezone(&tz);
    let mut last_tick = Instant::now();
    // When the last change signal arrived, if a reload is still due.
    let mut pending_since: Option<Instant> = None;
    let mut watch_ended = EndedWatch::new(&app.snapshot.config);
    let size = terminal.size()?;
    app.resize(size.width, size.height);
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let mut ticked = false;
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    match app.key_at(k, now()) {
                        Outcome::Quit => return Ok(()),
                        Outcome::Continue => {}
                        Outcome::ForgetList(preview) => {
                            if syncing {
                                app.message = Some(
                                    "sync already running; retry :forget-list when it finishes"
                                        .into(),
                                );
                            } else {
                                syncing = true;
                                start_forget(&app.snapshot.config, preview, &sync_tx);
                            }
                        }
                        Outcome::Sync { force_delete } => {
                            if syncing {
                                app.message = Some("sync already running".into());
                            } else {
                                syncing = true;
                                start_sync(&app.snapshot.config.vdir, force_delete, &sync_tx);
                            }
                        }
                        Outcome::SyncSwitch(on) => {
                            let cfg = &app.snapshot.config;
                            app.message = Some(if cfg.sync.is_none() {
                                "sync is not configured".into()
                            } else {
                                match crate::cli::sync::set_switch(cfg, Some(&cfg.vdir), on) {
                                    Ok(()) => crate::cli::sync::switch_message(on).into(),
                                    Err(e) => format!("cannot switch automatic sync: {e}"),
                                }
                            });
                            refresh_sync(app);
                        }
                        Outcome::Write(req) => {
                            // A save during a sync is queued at once, never waited for.
                            let result = writer.run(app.write_snapshot(), &req, Utc::now());
                            if result.is_err() {
                                app.write_refused();
                            }
                            // Reload after failures too: a stale write means the vdir moved on.
                            reload(app, root);
                            refresh_sync(app);
                            app.write_finished(result.map_err(|e| e.message()));
                        }
                        Outcome::EditNotes(notes) => {
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
                        Outcome::Resolve { id, keep } => {
                            let cfg = &app.snapshot.config;
                            let result = resolve(cfg, &id, keep, Utc::now());
                            // Same rule as `tdo conflicts resolve`.
                            if let Ok(r) = &result {
                                let recorded = *r == Resolution::Recorded;
                                if let Some(t) = after_resolve(cfg, recorded, Utc::now()) {
                                    crate::cli::sync::spawn(cfg, Some(&cfg.vdir), t);
                                }
                            }
                            app.message = Some(match result {
                                Ok(Resolution::Recorded) => "kept; applied on the next sync".into(),
                                Ok(_) => "kept".into(),
                                Err(e) => format!("resolve failed: {e}"),
                            });
                            reload(app, root);
                            refresh_sync(app);
                        }
                    }
                    ticked = true;
                }
                Event::Paste(text) => {
                    app.paste(&text);
                    ticked = true;
                }
                Event::Resize(w, h) => {
                    app.resize(w, h);
                    ticked = true;
                }
                _ => {}
            }
        }
        for message in sync_rx.try_iter() {
            syncing = false;
            finish_sync(app, root, message);
        }
        if let Some((_, rx)) = &watcher
            && rx.try_iter().count() > 0
        {
            pending_since = Some(Instant::now());
        }
        poll_ended(&mut watch_ended, app, writer, root);
        // Wait for a quiet DEBOUNCE after the last change, so a sync batch reloads once.
        if pending_since.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            pending_since = None;
            reload(app, root);
            refresh_sync(app);
        }
        app.expire_message(now());
        if ticked || last_tick.elapsed() >= TICK_EVERY {
            app.tick(now());
            if last_tick.elapsed() >= TICK_EVERY {
                refresh_sync(app);
            }
            last_tick = Instant::now();
        }
    }
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

/// Reload under the shared vdir lock (see [`crate::cli::load_vdir`]).
fn reload(app: &mut App, root: &Path) {
    let _lock = match crate::cli::reader_lock(&app.snapshot.config) {
        Ok(l) => l,
        Err(e) => return app.reload_failed(e),
    };
    match twydo_core::vdir::load(root) {
        Ok(report) => {
            let snapshot = queue::view(report, app.snapshot.config.clone());
            app.reload(snapshot);
        }
        Err(e) => app.reload_failed(e),
    }
}

/// Notices that some sync run (background or manual) has ended since the last look.
struct EndedWatch {
    last: Option<String>,
}

impl EndedWatch {
    fn new(cfg: &Config) -> EndedWatch {
        let last = twydo_core::sync::ended::read(cfg)
            .ok()
            .flatten()
            .map(|e| e.run_id);
        EndedWatch { last }
    }

    /// True once per new `run_id`; a read error or a missing file is not a change.
    fn changed(&mut self, cfg: &Config) -> bool {
        match twydo_core::sync::ended::read(cfg) {
            Ok(Some(e)) if self.last.as_deref() != Some(e.run_id.as_str()) => {
                self.last = Some(e.run_id);
                true
            }
            _ => false,
        }
    }
}

/// Reload when any sync has ended, whether or not the watcher saw the files change. Queued
/// edits the sync left are written first, and refusals are shown.
fn poll_ended(watch: &mut EndedWatch, app: &mut App, writer: &mut Writer, root: &Path) {
    if watch.changed(&app.snapshot.config) {
        drain_and_reload(app, writer, root);
    }
}

/// Write the queued edits if the vdir is free, reload, and show the refusals of edits that
/// were not saved (the drain's, or a sync's that nobody has seen yet).
fn drain_and_reload(app: &mut App, writer: &mut Writer, root: &Path) {
    let refusals = writer.drain(Utc::now());
    reload(app, root);
    refresh_sync(app);
    if !refusals.is_empty() {
        let shown = refusals.join(" · ");
        app.message = Some(match app.message.take() {
            Some(m) if m != crate::app::SYNC_DONE => format!("{shown} · {m}"),
            _ => shown,
        });
    }
}

/// Re-read the sync summary; on a read error keep what is shown.
fn refresh_sync(app: &mut App) {
    if let Ok(s) = twydo_core::sync::summary(&app.snapshot.config) {
        app.set_sync(s);
    }
}

/// Run `tdo sync` as a child on a thread. The loop gets stderr (or `sync done`)
/// through `tx`, so the UI never blocks.
fn start_sync(vdir: &Path, force_delete: Option<String>, tx: &mpsc::Sender<String>) {
    let vdir = vdir.to_path_buf();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let message = run_sync_child(&vdir, force_delete.as_deref())
            .unwrap_or_else(|e| format!("sync failed to start: {e}"));
        let _ = tx.send(message);
    });
}

fn run_sync_child(vdir: &Path, force_delete: Option<&str>) -> io::Result<String> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--vdir").arg(vdir).arg("sync");
    if let Some(list) = force_delete {
        cmd.arg("--force-delete").arg(list);
    }
    let out = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<_> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    Ok(if lines.is_empty() {
        crate::app::SYNC_DONE.to_string()
    } else {
        lines.join(" · ")
    })
}

fn start_forget(
    cfg: &twydo_core::config::Config,
    preview: twydo_core::retired::Preview,
    tx: &mpsc::Sender<String>,
) {
    let cfg = cfg.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let message = match twydo_core::retired::forget(&cfg, &preview, Utc::now()) {
            Err(e) => format!("list was not removed: {e}"),
            Ok(b) => {
                let saved = format!(
                    "preserved {:?} as {}; recover with :recover {} <destination-list>",
                    b.name, b.id, b.id
                );
                match twydo_core::sync::run::run(&cfg, &Default::default(), Utc::now()) {
                    Ok(r) if r.transient.is_none() => saved,
                    Ok(r) => format!("{saved} · {}", r.transient.unwrap_or_default()),
                    Err(e) => format!("{saved} · sync failed: {e}"),
                }
            }
        };
        let _ = tx.send(message);
    });
}

/// Apply the result delivered by the manual sync worker.
fn finish_sync(app: &mut App, root: &Path, message: String) {
    // Sync may have applied changes even if it failed. Do not rely on watcher delivery.
    // Reload before reporting, so a failed reload cannot hide what the sync said.
    reload(app, root);
    refresh_sync(app);
    app.sync_finished(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::Request;
    use twydo_core::compat::Matrix;
    use twydo_core::edit::{Change, Op};
    use twydo_core::store::fsio::VdirLock;
    use twydo_core::views::{Snapshot, View};

    const FORCE_DELETE: &str = "list \"inbox\" was emptied on the server; to apply that here run: tdo sync --force-delete \"inbox\"";

    /// An app over a vdir with one synthetic task in `inbox`, showing that list.
    fn inbox_app(dir: &Path) -> (App, PathBuf, PathBuf) {
        let root = dir.join("vdir");
        let list = root.join("inbox");
        std::fs::create_dir_all(&list).unwrap();
        let file = list.join("task.ics");
        std::fs::write(
            &file,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-task\r\nSUMMARY:Synthetic task\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let mut cfg = Config::with_vdir(root.clone());
        cfg.state_dir = Some(dir.join("state"));
        let snapshot = Snapshot::new(twydo_core::vdir::load(&root).unwrap(), cfg);
        let mut app = App::new(snapshot, Utc::now().with_timezone(&chrono_tz::UTC));
        app.view = View::List("inbox".into());
        assert_eq!(app.visible_rows().len(), 1);
        (app, root, file)
    }

    #[test]
    fn sync_completion_refreshes_an_emptied_list_without_watcher_events() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, file) = inbox_app(dir.path());

        // The sync child applied an empty server list. No watcher is running.
        std::fs::remove_file(file).unwrap();
        finish_sync(&mut app, &root, crate::app::SYNC_DONE.into());

        assert!(app.visible_rows().is_empty());
        assert!(app.snapshot.tasks.is_empty());
        assert_eq!(app.view, View::List("inbox".into()));
        assert_eq!(app.message.as_deref(), Some(crate::app::SYNC_DONE));
    }

    #[test]
    fn a_background_sync_end_reloads_without_watcher_events() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, file) = inbox_app(dir.path());
        let cfg = app.snapshot.config.clone();
        let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        let mut watch = EndedWatch::new(&cfg);
        let mut writer = Writer::open_with(&cfg, Matrix::embedded().clone(), Box::new(|_, _| {}));

        // No event yet: nothing reloads.
        let saved = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        poll_ended(&mut watch, &mut app, &mut writer, &root);
        assert_eq!(app.visible_rows().len(), 1);

        twydo_core::sync::ended::write(&state, Utc::now()).unwrap();
        poll_ended(&mut watch, &mut app, &mut writer, &root);
        assert!(app.visible_rows().is_empty());

        // The same event does not reload again.
        std::fs::write(&file, saved).unwrap();
        poll_ended(&mut watch, &mut app, &mut writer, &root);
        assert!(app.visible_rows().is_empty());
    }

    #[test]
    fn a_failed_sync_keeps_its_message_after_the_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, _) = inbox_app(dir.path());

        finish_sync(&mut app, &root, FORCE_DELETE.into());

        assert_eq!(app.message.as_deref(), Some(FORCE_DELETE));
        app.expire_message(Utc::now().with_timezone(&chrono_tz::UTC) + chrono::Duration::hours(1));
        assert_eq!(app.message.as_deref(), Some(FORCE_DELETE), "errors stay");
    }

    #[test]
    fn a_failed_reload_does_not_hide_the_sync_message() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, _) = inbox_app(dir.path());
        std::fs::remove_dir_all(&root).unwrap();

        finish_sync(&mut app, &root, FORCE_DELETE.into());
        let shown = app.message.clone().unwrap();
        assert!(shown.starts_with(FORCE_DELETE), "{shown}");
        assert!(shown.contains("reload failed: "), "{shown}");

        // A reload that works later must not clear the instruction.
        let (_, root, _) = inbox_app(dir.path());
        reload(&mut app, &root);
        assert_eq!(app.message.as_deref(), Some(shown.as_str()));
    }

    #[test]
    fn a_failed_reload_after_a_good_sync_shows_the_reload_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, _) = inbox_app(dir.path());
        std::fs::remove_dir_all(&root).unwrap();

        finish_sync(&mut app, &root, crate::app::SYNC_DONE.into());
        let shown = app.message.clone().unwrap();
        assert!(shown.starts_with("reload failed: "), "{shown}");
        app.expire_message(Utc::now().with_timezone(&chrono_tz::UTC) + chrono::Duration::hours(1));
        assert_eq!(app.message.as_deref(), Some(shown.as_str()), "errors stay");
    }

    fn edit_title(title: &str) -> Request {
        Request::Apply(Op::Edit {
            uid: "synthetic-task".into(),
            changes: vec![Change::Summary(title.into())],
        })
    }

    fn quiet_writer(cfg: &Config) -> Writer {
        Writer::open_with(cfg, Matrix::embedded().clone(), Box::new(|_, _| {}))
    }

    /// The state dir of the app's vdir, as a sync would hold its lock.
    fn sync_lock(cfg: &Config) -> VdirLock {
        let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        VdirLock::shared(&state).unwrap()
    }

    #[test]
    fn queued_edit_shows_after_save() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, file) = inbox_app(dir.path());
        let cfg = app.snapshot.config.clone();
        let mut writer = quiet_writer(&cfg);
        let _sync = sync_lock(&cfg);
        let before = std::fs::read(&file).unwrap();

        let result = writer.run(app.write_snapshot(), &edit_title("Renamed"), Utc::now());
        assert!(result.is_ok(), "{result:?}");
        reload(&mut app, &root);

        let row = app.visible_rows()[0].task;
        assert_eq!(app.snapshot.tasks[row].summary, "Renamed");
        assert!(app.snapshot.is_pending(row), "the row is marked queued");
        assert_eq!(std::fs::read(&file).unwrap(), before, "nothing written yet");
    }

    #[test]
    fn sync_ended_drains_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, file) = inbox_app(dir.path());
        let cfg = app.snapshot.config.clone();
        let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        let mut writer = quiet_writer(&cfg);
        let mut watch = EndedWatch::new(&cfg);
        let sync = sync_lock(&cfg);
        writer
            .run(app.write_snapshot(), &edit_title("Renamed"), Utc::now())
            .unwrap();
        reload(&mut app, &root);
        assert!(app.snapshot.is_pending(app.visible_rows()[0].task));

        drop(sync);
        twydo_core::sync::ended::write(&state, Utc::now()).unwrap();
        poll_ended(&mut watch, &mut app, &mut writer, &root);

        let row = app.visible_rows()[0].task;
        assert!(!app.snapshot.is_pending(row), "written, so no marker");
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("SUMMARY:Renamed")
        );
    }

    #[test]
    fn startup_shows_unshown_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, root, file) = inbox_app(dir.path());
        let cfg = app.snapshot.config.clone();
        let mut writer = quiet_writer(&cfg);
        let sync = sync_lock(&cfg);
        writer
            .run(app.write_snapshot(), &edit_title("Renamed"), Utc::now())
            .unwrap();
        // Another device changes the task; the session ends before the queue is written.
        let old = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, old.replace("Synthetic task", "From phone")).unwrap();
        drop(sync);

        let mut next = quiet_writer(&cfg);
        drain_and_reload(&mut app, &mut next, &root);
        assert_eq!(
            app.message.as_deref(),
            Some("Not saved: \"Synthetic task\" changed on another device")
        );
        assert_eq!(app.snapshot.tasks[0].summary, "From phone");
    }
}
