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
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use twydo_core::sync::resolve::{Resolution, resolve};
use twydo_core::views::Snapshot;

use crate::app::{App, Outcome};
use crate::ui;

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
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &root, tz);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
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
    set_page(app, terminal.size()?.height);
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let mut ticked = false;
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    match app.handle_key(k) {
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
                        Outcome::Resolve { id, keep } => {
                            let cfg = &app.snapshot.config;
                            let result = resolve(cfg, &id, keep, Utc::now());
                            // Same rule as `tdo conflicts resolve`: sync after a resolve
                            // that worked, when `[sync] after_edit` is on.
                            if result.is_ok() && cfg.sync.as_ref().is_some_and(|s| s.after_edit) {
                                crate::cli::sync::spawn_background(cfg, Some(&cfg.vdir));
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
                Event::Resize(_, h) => {
                    set_page(app, h);
                    ticked = true;
                }
                _ => {}
            }
        }
        for message in sync_rx.try_iter() {
            syncing = false;
            app.sync_finished(message);
            refresh_sync(app);
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
        if ticked || last_tick.elapsed() >= TICK_EVERY {
            app.tick(now());
            if last_tick.elapsed() >= TICK_EVERY {
                refresh_sync(app);
            }
            last_tick = Instant::now();
        }
    }
}

/// Half the task pane height: the screen minus the status line and the pane's two borders.
fn set_page(app: &mut App, height: u16) {
    app.page = (height.saturating_sub(3) / 2).max(1) as usize;
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
        .map_or_else(|| "sync done".to_string(), str::to_string))
}
