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
use twydo_core::store::fsio::VdirLock;
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
                    if app.handle_key(k) == Outcome::Quit {
                        return Ok(());
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
        if let Some((_, rx)) = &watcher
            && rx.try_iter().count() > 0
        {
            pending_since = Some(Instant::now());
        }
        // Wait for a quiet DEBOUNCE after the last change, so a sync batch reloads once.
        if pending_since.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            pending_since = None;
            reload(app, root);
        }
        if ticked || last_tick.elapsed() >= TICK_EVERY {
            app.tick(now());
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
    let state = app.snapshot.config.state_dir();
    let _lock = match VdirLock::reader(&state) {
        Ok(l) => l,
        Err(e) => return app.reload_failed(format!("cannot lock {}: {e}", state.display())),
    };
    match twydo_core::vdir::load(root) {
        Ok(report) => {
            let snapshot = Snapshot::new(report, app.snapshot.config.clone());
            app.reload(snapshot);
        }
        Err(e) => app.reload_failed(e),
    }
}
