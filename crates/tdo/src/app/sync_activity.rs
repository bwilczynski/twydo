//! Sync progress, independent of edit messages and the persisted summary.
use std::time::Instant;

#[derive(Default)]
pub struct SyncActivity {
    started: Option<Instant>,
    frame: usize,
}

impl SyncActivity {
    pub fn observe(&mut self, running: bool, now: Instant) {
        if running {
            let started = *self.started.get_or_insert(now);
            self.frame = (now.saturating_duration_since(started).as_millis() / 250 % 10) as usize;
        } else {
            self.started = None;
        }
    }

    pub fn label(&self) -> Option<String> {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        self.started
            .map(|_| format!("{} Syncing…", FRAMES[self.frame]))
    }
}
