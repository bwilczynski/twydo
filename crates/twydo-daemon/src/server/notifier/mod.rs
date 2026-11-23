//! The reminder notifier (reminder spec): one thread that wakes at the instants the pure
//! planner ([`twydo_core::remind::plan`]) names, delivers what is due through a [`Sink`] and
//! records it in the sent-state file. It never writes a task file, and what it logs or reports
//! names keys and UIDs, never titles.
//!
//! The thread never waits longer than [`MAX_WAIT`] and re-reads the clock on every wake, so a
//! laptop that wakes from sleep catches up within that time. A poke (after a publish, a sync's
//! end, a configuration reload) wakes it at once. It holds none of its own locks while it
//! plans, runs a command or touches the publisher; see the lock order in
//! [`super::writes`].
//!
//! Nothing is delivered before the first sync after a start or a wake from sleep (the *hold*,
//! see [`Hold`]): the snapshot from before the machine was off is stale, and the sync that
//! follows brings what changed on the phone. The hold applies only when a sync will come.
//!
//! A delivery that fails is reported in `status.notifier_error` and retried after a backoff of
//! its own key (5 s doubling to 300 s, reset by a success); the other deliveries of the pass go
//! on. A pass that panics is reported, logged and retried after the same kind of backoff; the
//! thread never ends because of one. A delivery whose record cannot be saved stays
//! recorded in memory, so it is not repeated while the daemon runs; only a crash between the
//! delivery and the record repeats one (a duplicate beats a lost reminder).

pub mod sink;
pub mod state;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use twydo_core::config::Config;
use twydo_core::remind::{self, Settings};
use twydo_core::views::{self, Snapshot, View};

pub use sink::{CommandSink, PlatformSink, Sink};
use state::SentState;

use super::Shared;
use super::schedule::Clock;
use crate::protocol::rpc::{ErrorKind, RpcError};

/// The longest the notifier waits before it looks at the clock again.
pub const MAX_WAIT: Duration = Duration::from_secs(5);

/// The first retry delay after a failed delivery; it doubles up to [`MAX_BACKOFF`].
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// A pass that sees the clock moved forward by more than this since the previous pass (the
/// wait cap is 5 s) takes it for a sleep or a jump and starts a hold.
pub const WAKE_GAP: chrono::Duration = chrono::Duration::seconds(60);

/// The longest a hold lasts, on the notifier's clock.
pub const HOLD_MAX: chrono::Duration = chrono::Duration::seconds(120);

/// The message of a panicked pass.
const PANIC_ERROR: &str = "the reminder planner failed; see the daemon log";

struct Inner {
    poked: bool,
    stopped: bool,
    /// How many syncs have finished since the notifier started.
    syncs: u64,
}

struct Core {
    inner: Mutex<Inner>,
    cv: Condvar,
    clock: Arc<dyn Clock>,
    /// The clock when the notifier started: where the first hold begins.
    start: DateTime<Utc>,
    /// The platform sink; `None` leaves the notifier idle unless a command is configured.
    sink: Option<Arc<dyn Sink>>,
    /// The time limit of a configured command.
    timeout: Duration,
}

impl Core {
    fn sink_for(&self, cfg: &Config) -> Option<Arc<dyn Sink>> {
        match &cfg.notifications.command {
            Some(argv) => Some(Arc::new(CommandSink {
                argv: argv.clone(),
                timeout: self.timeout,
            })),
            None => self.sink.clone(),
        }
    }
}

/// The notifier thread and its controls.
pub struct Notifier {
    core: Arc<Core>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Notifier {
    /// Starts the thread. `sink` is the platform sink (`None`: idle unless
    /// `[notifications] command` is set); `timeout` bounds a configured command.
    pub fn start(
        shared: Weak<Shared>,
        clock: Arc<dyn Clock>,
        sink: Option<Arc<dyn Sink>>,
        timeout: Duration,
    ) -> Notifier {
        let core = Arc::new(Core {
            inner: Mutex::new(Inner {
                poked: false,
                stopped: false,
                syncs: 0,
            }),
            cv: Condvar::new(),
            start: clock.now(),
            clock,
            sink,
            timeout,
        });
        let handle = {
            let core = Arc::clone(&core);
            std::thread::spawn(move || run(&core, &shared))
        };
        Notifier {
            core,
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Wakes the thread to plan again.
    pub fn poke(&self) {
        let mut g = self.core.inner.lock().unwrap();
        g.poked = true;
        self.core.cv.notify_all();
    }

    /// A sync has finished (whatever its result): ends a hold that started before it. Like
    /// [`Notifier::poke`] it only sets a flag under the notifier's own mutex.
    pub fn sync_finished(&self) {
        let mut g = self.core.inner.lock().unwrap();
        g.syncs += 1;
        g.poked = true;
        self.core.cv.notify_all();
    }

    /// Stops the thread and waits for it (a delivery in flight ends within its time limit).
    /// Idempotent.
    pub fn stop(&self) {
        {
            let mut g = self.core.inner.lock().unwrap();
            g.stopped = true;
            self.core.cv.notify_all();
        }
        if let Some(h) = self.handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    /// The sink in force for `cfg`: its `command` if set, else the option's sink.
    pub fn sink_for(&self, cfg: &Config) -> Option<Arc<dyn Sink>> {
        self.core.sink_for(cfg)
    }
}

/// `now` plus `d`; an overflow means the end of time.
fn later(now: DateTime<Utc>, d: Duration) -> DateTime<Utc> {
    chrono::Duration::from_std(d)
        .ok()
        .and_then(|d| now.checked_add_signed(d))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// `text` with every control character except a newline (a NUL fails `Command::spawn`) made a
/// space, so every sink gets text it can pass on.
fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect()
}

/// Today's open count at `at`, as `views.get` counts it (the phone's icon badge).
fn today_open(snap: &Snapshot, at: DateTime<Utc>) -> usize {
    views::counts(snap, &View::Today, at.with_timezone(&snap.tz())).total
}

fn cleaned(d: &remind::Delivery) -> remind::Delivery {
    remind::Delivery {
        title: clean_text(&d.title),
        body: clean_text(&d.body),
        ..d.clone()
    }
}

/// `err` with the delivery's title and every non-empty line of its body replaced by
/// `<text>`, also as a sink writes them (HTML-escaped, or with the backslash of
/// `terminal-notifier`): a sink's error may echo its arguments, and neither the log nor the
/// status may carry task text.
fn scrub(err: &str, d: &remind::Delivery) -> String {
    let mut secrets: Vec<String> = Vec::new();
    for raw in std::iter::once(d.title.as_str())
        .chain(d.body.lines())
        .filter(|s| !s.trim().is_empty())
    {
        secrets.push(format!("\\{raw}"));
        secrets.push(sink::escape_markup(raw));
        secrets.push(raw.to_string());
    }
    // Longest first, so a line is replaced whole before a title inside it is.
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = err.to_string();
    for secret in secrets {
        out = out.replace(&secret, "<text>");
    }
    out
}

/// A time to try again, and the delay it was set with. The time is clamped to at most `now`
/// plus the delay, so a clock that moved backwards cannot park a retry hours ahead.
#[derive(Debug, Clone, Copy)]
struct Retry {
    at: DateTime<Utc>,
    delay: Duration,
}

impl Retry {
    fn new(now: DateTime<Utc>, delay: Duration) -> Retry {
        Retry {
            at: later(now, delay),
            delay,
        }
    }

    fn effective(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        self.at.min(later(now, self.delay))
    }

    /// The delay after this one.
    fn next_delay(&self) -> Duration {
        (self.delay * 2).min(MAX_BACKOFF)
    }
}

/// Deliveries are held back: since when, and how many syncs had finished by then. The hold
/// ends when one more has, or [`HOLD_MAX`] has passed.
#[derive(Debug, Clone, Copy)]
struct Hold {
    since: DateTime<Utc>,
    syncs: u64,
}

/// What the thread remembers between wakes.
struct Memory {
    sent: SentState,
    /// Keys whose delivery failed: when to try each again.
    retries: BTreeMap<String, Retry>,
    hold: Option<Hold>,
    /// The clock at the end of the previous pass, to see a wake.
    last: Option<(DateTime<Utc>, Instant)>,
    /// A pass panicked: nothing is planned before then.
    panicked: Option<Retry>,
    /// Why the last delivery failed, why the last save failed (cleared by a success), and
    /// whether the last pass panicked.
    deliver_error: Option<String>,
    save_error: Option<String>,
    panic_error: Option<String>,
    /// What the status last said.
    reported: Option<String>,
}

impl Memory {
    fn error(&self) -> Option<String> {
        self.panic_error
            .clone()
            .or_else(|| self.deliver_error.clone())
            .or_else(|| self.save_error.clone())
    }
}

/// Records `key` as handled; a failed save keeps it in memory and is reported.
fn record(mem: &mut Memory, key: &str, now: DateTime<Utc>, settings: &Settings) {
    match mem.sent.record(key, now, settings.catch_up) {
        Ok(()) => mem.save_error = None,
        Err(e) => {
            eprintln!("tdo daemon: cannot save reminder state: {e}");
            mem.save_error = Some(format!("cannot save reminder state: {e}"));
        }
    }
}

/// Is a sync going to come (`[sync]` configured, automatic sync on, not backing off)? Only
/// then is there anything to hold for.
fn sync_will_come(shared: &Shared) -> bool {
    shared.cfg.sync.is_some() && !super::jobs::skip_background(shared)
}

/// Did the machine sleep, or the clock jump forward, between two passes? `wall_gap` is the
/// notifier clock's difference between them and `elapsed` the monotonic time that really passed.
/// A pass's own duration (hung delivery commands) and its wait are in both and cancel out;
/// only the surplus counts. The monotonic clock is used here, unlike everywhere else, because
/// it does not run while the machine sleeps (Linux, macOS) and the injected clock cannot tell
/// a slow pass from a sleep; a test clock moves only by the test's hand, so `elapsed` is tiny.
fn is_wake(wall_gap: chrono::Duration, elapsed: Duration) -> bool {
    let elapsed = chrono::Duration::from_std(elapsed).unwrap_or(chrono::Duration::MAX);
    wall_gap
        .checked_sub(&elapsed)
        .is_some_and(|extra| extra > WAKE_GAP)
}

/// The remaining hold: `Some(wait)` while deliveries are held back. A sync that finished
/// since the hold began ends it, as do [`HOLD_MAX`] and a sync that will not come
/// (`will_come` is only asked when nothing else ended the hold).
fn hold_wait(
    hold: &mut Hold,
    now: DateTime<Utc>,
    syncs: u64,
    will_come: impl FnOnce() -> bool,
) -> Option<Duration> {
    // A clock stepped backward (an NTP correction) leaves `since` ahead of `now`: count the
    // bound from the corrected time.
    hold.since = hold.since.min(now);
    if syncs > hold.syncs || now.signed_duration_since(hold.since) >= HOLD_MAX || !will_come() {
        return None;
    }
    let left = hold
        .since
        .checked_add_signed(HOLD_MAX)
        .and_then(|end| (end - now).to_std().ok())
        .unwrap_or_default();
    Some(left.min(MAX_WAIT))
}

fn held(shared: &Shared, mem: &mut Memory, now: DateTime<Utc>, syncs: u64) -> Option<Duration> {
    let hold = mem.hold.as_mut()?;
    let wait = hold_wait(hold, now, syncs, || sync_will_come(shared));
    if wait.is_none() {
        mem.hold = None;
    }
    wait
}

/// One wake: plans and delivers. Returns how long to wait.
fn pass(core: &Core, shared: &Shared, mem: &mut Memory, now: DateTime<Utc>) -> Duration {
    let cfg = shared.publisher.config();
    let syncs = core.inner.lock().unwrap().syncs;
    // A forward jump of the clock: the machine slept (a backward one starts nothing). It
    // restarts a hold that is running too, counting only syncs that end from now on.
    if mem
        .last
        .is_some_and(|(prev, at)| is_wake(now.signed_duration_since(prev), at.elapsed()))
    {
        mem.hold = Some(Hold { since: now, syncs });
    }
    let sink = core.sink_for(&cfg).filter(|_| cfg.notifications.enabled);
    let Some(sink) = sink else {
        mem.retries.clear();
        mem.deliver_error = None;
        mem.save_error = None;
        report(shared, mem);
        return MAX_WAIT;
    };
    if let Some(wait) = held(shared, mem, now, syncs) {
        report(shared, mem);
        return wait;
    }
    let settings = Settings::from_config(&cfg);
    let (_, snap) = shared.publisher.current();
    let plan = remind::plan(&snap.snap, mem.sent.sent(), now, &settings);
    let keys: BTreeSet<&str> = plan.due.iter().map(|d| d.key.as_str()).collect();
    mem.retries.retain(|k, _| keys.contains(k.as_str()));
    let mut failure: Option<String> = None;
    let open = (!plan.due.is_empty()).then(|| today_open(&snap.snap, now));
    for d in &plan.due {
        if mem
            .retries
            .get(&d.key)
            .is_some_and(|r| now < r.effective(now))
        {
            continue;
        }
        let d = remind::Delivery {
            today_open: open,
            ..cleaned(d)
        };
        match sink.deliver(&d) {
            Ok(()) => {
                mem.retries.remove(&d.key);
                record(mem, &d.key, now, &settings);
            }
            Err(e) => {
                let e = scrub(&e, &d);
                eprintln!("tdo daemon: cannot deliver reminder {}: {e}", d.key);
                failure = Some(format!("cannot deliver reminders: {e}"));
                let delay = mem
                    .retries
                    .get(&d.key)
                    .map_or(FIRST_BACKOFF, Retry::next_delay);
                mem.retries.insert(d.key.clone(), Retry::new(now, delay));
            }
        }
    }
    // The most recent failure shows until no key is failing any more.
    if failure.is_some() {
        mem.deliver_error = failure;
    } else if mem.retries.is_empty() {
        mem.deliver_error = None;
    }
    // An empty digest is handled without delivering anything.
    for key in &plan.skip {
        record(mem, key, now, &settings);
    }
    report(shared, mem);
    let mut wait = MAX_WAIT;
    if let Ok(d) = (plan.next_wake - now).to_std() {
        wait = wait.min(d);
    }
    for r in mem.retries.values() {
        if let Ok(d) = (r.effective(now) - now).to_std() {
            wait = wait.min(d);
        }
    }
    wait
}

/// Tells the status when the notifier's error changed.
fn report(shared: &Shared, mem: &mut Memory) {
    let now = mem.error();
    if now != mem.reported {
        mem.reported = now.clone();
        shared.publisher.set_notifier_error(now);
        shared.publisher.refresh_status();
    }
}

fn run(core: &Core, shared: &Weak<Shared>) {
    let (sent, why) = {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        SentState::load(&shared.state_dir)
    };
    if let Some(why) = why {
        eprintln!("tdo daemon: reminder state starts empty: {why}");
    }
    let mut mem = Memory {
        sent,
        retries: BTreeMap::new(),
        // A start holds until the first sync (when one will come).
        hold: Some(Hold {
            since: core.start,
            syncs: 0,
        }),
        last: None,
        panicked: None,
        deliver_error: None,
        save_error: None,
        panic_error: None,
        reported: None,
    };
    loop {
        let (wait, now, started) = {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            if shared.is_stopping() || core.inner.lock().unwrap().stopped {
                return;
            }
            let now = core.clock.now();
            let started = Instant::now();
            let wait = match mem.panicked {
                Some(r) if now < r.effective(now) => (r.effective(now) - now)
                    .to_std()
                    .unwrap_or_default()
                    .min(MAX_WAIT),
                _ => {
                    let before = mem.panicked.take();
                    mem.panic_error = None;
                    match catch_unwind(AssertUnwindSafe(|| pass(core, &shared, &mut mem, now))) {
                        Ok(wait) => wait,
                        Err(_) => {
                            let delay = before.map_or(FIRST_BACKOFF, |r| r.next_delay());
                            eprintln!(
                                "tdo daemon: the reminder planner panicked; retrying in {delay:?}"
                            );
                            mem.panicked = Some(Retry::new(now, delay));
                            mem.panic_error = Some(PANIC_ERROR.to_string());
                            report(&shared, &mut mem);
                            delay.min(MAX_WAIT)
                        }
                    }
                }
            };
            (wait, now, started)
        };
        // The clock as the pass read it: a clock read after the pass could already include a
        // jump that the next pass must see as a gap.
        mem.last = Some((now, started));
        let g = core.inner.lock().unwrap();
        let (mut g, _) = core
            .cv
            .wait_timeout_while(g, wait, |i| !i.poked && !i.stopped)
            .unwrap();
        if g.stopped {
            return;
        }
        g.poked = false;
    }
}

impl Shared {
    /// Wakes the notifier, if it runs.
    pub fn poke_notifier(&self) {
        if let Some(n) = self.notifier.lock().unwrap().as_ref() {
            n.poke();
        }
    }

    /// A sync has finished: wakes the notifier, and ends its hold if one started before.
    pub fn sync_finished_notifier(&self) {
        if let Some(n) = self.notifier.lock().unwrap().as_ref() {
            n.sync_finished();
        }
    }

    /// `notify.test`: delivers a fixed message through the sink in force for the current
    /// configuration, whether or not reminders are enabled, and records nothing.
    pub fn notify_test(&self) -> Result<serde_json::Value, RpcError> {
        let cfg = self.publisher.config();
        // The sink is cloned out so no lock is held while it runs.
        let sink = self
            .notifier
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|n| n.sink_for(&cfg));
        let Some(sink) = sink else {
            return Err(RpcError::refused(
                ErrorKind::Invalid,
                "notifications are not available in this daemon",
            ));
        };
        let d = remind::Delivery {
            key: "test".into(),
            uid: None,
            title: "Twydo".into(),
            body: "Notifications work.".into(),
            today_open: Some(today_open(
                &self.publisher.current().1.snap,
                self.clock.now(),
            )),
        };
        match sink.deliver(&d) {
            Ok(()) => Ok(serde_json::json!({})),
            Err(e) => Err(RpcError::refused(ErrorKind::Invalid, scrub(&e, &d))),
        }
    }

    /// Starts the notifier unless the daemon is stopping (checked under the slot's lock,
    /// which teardown takes after stop is set).
    pub(super) fn start_notifier(
        self: &Arc<Self>,
        clock: Arc<dyn Clock>,
        sink: Option<Arc<dyn Sink>>,
        timeout: Duration,
    ) {
        let mut slot = self.notifier.lock().unwrap();
        if self.is_stopping() || slot.is_some() {
            return;
        }
        *slot = Some(Notifier::start(Arc::downgrade(self), clock, sink, timeout));
    }

    /// Stops the notifier (teardown, before the state directory lock is released).
    pub(super) fn stop_notifier(&self) {
        let n = self.notifier.lock().unwrap().take();
        if let Some(n) = n {
            n.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(title: &str, body: &str) -> remind::Delivery {
        remind::Delivery {
            key: "d:2030-01-10".into(),
            uid: None,
            title: title.into(),
            body: body.into(),
            today_open: None,
        }
    }

    #[test]
    fn scrub_removes_the_title_and_every_body_line() {
        let d = delivery("2 due today", "Buy zebra food\nCall the vet\n\n+1 more");
        let e = "sh failed: exit status: 1: 2 due today Buy zebra food Call the vet";
        let out = scrub(e, &d);
        assert!(!out.contains("zebra") && !out.contains("vet"), "{out}");
        assert!(out.contains("sh failed: exit status: 1"), "{out}");
    }

    #[test]
    fn scrub_removes_the_escaped_and_backslashed_forms() {
        let d = delivery("-Fish", "Fish & chips <now>\n[x]");
        let e = "boom: Fish &amp; chips &lt;now&gt; and \\-Fish and \\[x]";
        let out = scrub(e, &d);
        assert!(!out.contains("Fish") && !out.contains("chips"), "{out}");
        assert!(!out.contains("[x]") && !out.contains('\\'), "{out}");
    }

    #[test]
    fn control_characters_are_removed_from_notification_text() {
        let d = remind::Delivery {
            key: "k".into(),
            uid: None,
            title: "a\0b\u{7}c\td".into(),
            body: "line one\r\nline\0two\u{1b}[0m".into(),
            today_open: None,
        };
        let c = cleaned(&d);
        assert_eq!(c.title, "a b c d");
        assert_eq!(c.body, "line one \nline two [0m");
    }

    #[test]
    fn a_retry_time_is_clamped_after_a_clock_rollback() {
        use chrono::TimeZone;
        let at = Utc.with_ymd_and_hms(2030, 1, 10, 12, 0, 0).unwrap();
        let r = Retry::new(at, Duration::from_secs(5));
        let rolled_back = at - chrono::Duration::hours(3);
        assert_eq!(
            r.effective(rolled_back),
            rolled_back + chrono::Duration::seconds(5)
        );
        assert_eq!(r.effective(at), r.at);
        assert_eq!(r.next_delay(), Duration::from_secs(10));
    }

    #[test]
    fn a_slow_pass_is_not_a_wake_but_a_sleep_is() {
        let secs = chrono::Duration::seconds;
        let d = Duration::from_secs;
        assert!(!is_wake(secs(70), d(70)), "a slow pass");
        assert!(is_wake(secs(3 * 3600), d(5)), "a sleep");
        assert!(!is_wake(secs(-30), d(5)), "a backward step");
        assert!(!is_wake(secs(-30), d(0)));
        assert!(
            !is_wake(secs(65), d(5)),
            "60 s of surplus is not over the gap"
        );
        assert!(is_wake(secs(66), d(5)));
    }

    #[test]
    fn a_backward_step_keeps_the_hold_bounded() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2030, 1, 10, 4, 0, 0).unwrap();
        let mut hold = Hold {
            since: now + chrono::Duration::minutes(10),
            syncs: 0,
        };
        assert!(hold_wait(&mut hold, now, 0, || true).is_some());
        assert_eq!(hold.since, now, "pulled back to the corrected time");
        let end = now + HOLD_MAX;
        assert!(hold_wait(&mut hold, end, 0, || true).is_none());
    }

    #[test]
    fn a_finished_sync_ends_a_hold_even_when_a_sync_will_come() {
        use chrono::TimeZone;
        let since = Utc.with_ymd_and_hms(2030, 1, 10, 4, 0, 0).unwrap();
        let mut hold = Hold { since, syncs: 3 };
        let now = since + chrono::Duration::seconds(10);
        assert!(
            hold_wait(&mut hold, now, 3, || true).is_some(),
            "no sync yet"
        );
        assert!(
            hold_wait(&mut hold, now, 4, || true).is_none(),
            "a sync ended"
        );
        let late = since + HOLD_MAX;
        assert!(
            hold_wait(&mut hold, late, 3, || true).is_none(),
            "the bound"
        );
        assert!(
            hold_wait(&mut hold, now, 3, || false).is_none(),
            "none will come"
        );
    }

    #[test]
    fn scrub_keeps_an_error_that_names_no_text() {
        let d = delivery("Ping", "");
        assert_eq!(scrub("cannot start x: boom", &d), "cannot start x: boom");
    }
}
