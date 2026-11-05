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
use twydo_core::sync::resolve::{Resolution, resolve};
use twydo_core::views::Snapshot;

use crate::app::{App, Outcome};
use crate::ui;
use crate::write::{Refused, Writer};

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
    // Saves made just before quitting still sync now, not on the next timer run.
    writer.flush_sync();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    writer: &mut Writer,
    root: &Path,
    tz: Tz,
) -> io::Result<()> {
    // Without a watcher the app still works; it just does not live-reload.
    let watcher = match watch(root) {
        Ok(w) => Some(w),
        Err(e) => {
            app.message = Some(format!("live reload unavailable: {e}"));
            None
        }
    };
    let (sync_tx, sync_rx) = mpsc::channel::<String>();
    let mut syncing = false;
    // One background sync at start, then the first summary.
    crate::cli::sync::spawn_background(&app.snapshot.config, Some(&app.snapshot.config.vdir));
    refresh_sync(app);
    let now = || Utc::now().with_timezone(&tz);
    let mut last_tick = Instant::now();
    // When the last change signal arrived, if a reload is still due.
    let mut pending_since: Option<Instant> = None;
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
                        Outcome::Sync { force_delete } => {
                            if syncing {
                                app.message = Some("sync already running".into());
                            } else {
                                syncing = true;
                                start_sync(&app.snapshot.config.vdir, force_delete, &sync_tx);
                            }
                        }
                        Outcome::Write(req) => {
                            // Waits up to write::LOCK_WAIT for a running sync.
                            let result = writer.run(app.write_snapshot(), &req, Utc::now());
                            // A retry after `y` must be checked against what this write saw,
                            // not against the reload below.
                            let base = matches!(result, Err(Refused::Busy { .. }))
                                .then(|| app.write_snapshot().clone());
                            if result.is_err() {
                                app.write_refused();
                            }
                            // Reload after failures too: a stale write means the vdir moved on.
                            reload(app, root);
                            refresh_sync(app);
                            match (result, base) {
                                (Err(Refused::Busy { running_for }), Some(base)) => {
                                    app.write_busy(req, running_for, base)
                                }
                                (other, _) => app.write_finished(other.map_err(|e| e.message())),
                            }
                        }
                        Outcome::StopSyncAndWrite(req) => {
                            // Waits up to write::STOP_WAIT for the sync to let go.
                            let result =
                                writer.stop_and_run(app.write_snapshot(), &req, Utc::now());
                            if result.is_err() {
                                app.write_refused();
                            }
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
                            // Same rule as `tdo conflicts resolve`: sync after a resolve
                            // that worked, when `[sync] after_edit` is on.
                            if result.is_ok() {
                                writer.sync_soon();
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
        // Wait for a quiet DEBOUNCE after the last change, so a sync batch reloads once.
        if pending_since.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            pending_since = None;
            reload(app, root);
            refresh_sync(app);
        }
        // The after-edit sync starts once saves pause (issue #19).
        writer.sync_if_quiet(Instant::now());
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
            let snapshot = Snapshot::new(report, app.snapshot.config.clone());
            app.reload(snapshot);
        }
        Err(e) => app.reload_failed(e),
    }
}

/// Re-read the sync summary; on a read error keep what is shown.
fn refresh_sync(app: &mut App) {
    if let Ok(s) = twydo_core::sync::summary(&app.snapshot.config) {
        app.set_sync(s);
    }
}

/// Run `tdo sync` as a child on a thread. The loop gets the first stderr line (or `sync done`)
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
    Ok(stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map_or_else(|| crate::app::SYNC_DONE.to_string(), str::to_string))
}

/// Apply the result delivered by the manual sync worker.
fn finish_sync(app: &mut App, root: &Path, message: String) {
    app.sync_finished(message);
    // Sync may have applied changes even if it failed. Do not rely on watcher delivery.
    reload(app, root);
    refresh_sync(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use twydo_core::config::Config;
    use twydo_core::views::View;

    #[test]
    fn sync_completion_refreshes_an_emptied_list_without_watcher_events() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vdir");
        let list = root.join("inbox");
        std::fs::create_dir_all(&list).unwrap();
        let file = list.join("task.ics");
        std::fs::write(
            &file,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-task\r\nSUMMARY:Synthetic task\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let mut cfg = Config::with_vdir(root.clone());
        cfg.state_dir = Some(dir.path().join("state"));
        let snapshot = Snapshot::new(twydo_core::vdir::load(&root).unwrap(), cfg);
        let mut app = App::new(snapshot, Utc::now().with_timezone(&chrono_tz::UTC));
        app.view = View::List("inbox".into());
        assert_eq!(app.visible_rows().len(), 1);

        // The sync child applied an empty server list. No watcher is running.
        std::fs::remove_file(file).unwrap();
        finish_sync(&mut app, &root, crate::app::SYNC_DONE.into());

        assert!(app.visible_rows().is_empty());
        assert!(app.snapshot.tasks.is_empty());
        assert_eq!(app.view, View::List("inbox".into()));
        assert_eq!(app.message.as_deref(), Some(crate::app::SYNC_DONE));
    }
}
