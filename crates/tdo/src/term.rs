//! Terminal loop: raw mode, key events, live reload of the vdir and the minute tick.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use chrono::Utc;
use chrono_tz::Tz;
use notify_debouncer_mini::notify::{self, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use twydo_core::views::Snapshot;

use crate::app::{App, Outcome};
use crate::ui;

const DEBOUNCE: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_secs(1);
const TICK_EVERY: Duration = Duration::from_secs(30);

/// Watch `root` recursively. One `()` arrives per debounced batch of changes. The debouncer
/// must be kept alive for as long as signals are wanted.
pub fn watch(root: &Path) -> notify::Result<(Debouncer<notify::RecommendedWatcher>, Receiver<()>)> {
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
        if matches!(&res, Ok(events) if !events.is_empty()) || res.is_err() {
            let _ = tx.send(());
        }
    })?;
    debouncer.watcher().watch(root, RecursiveMode::Recursive)?;
    Ok((debouncer, rx))
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

fn reload(app: &mut App, root: &Path) {
    match twydo_core::vdir::load(root) {
        Ok(report) => {
            let snapshot = Snapshot::new(report, app.snapshot.config.clone());
            app.reload(snapshot);
        }
        Err(e) => app.message = Some(format!("reload failed: {e}")),
    }
}
