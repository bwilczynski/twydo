# One writer for the vdir

Date: 2026-11-13 · Status: open questions decided 2026-11-13; awaiting approval.

## Intent

The daemon owns the vdir ([daemon design](2026-11-09-daemon-design.md), D1), but the core type
it writes through, `twydo_core::store::Store`, still has the shape it had when the TUI and
`tdo op` were separate processes that each opened their own. The daemon wraps that type
instead of owning one. This design replaces `Store` with three smaller pieces: a pure
planner, a single writer the daemon owns, and a per-session undo history that is plain data.

It is a refactor. It changes no gate decision, no on-disk format and no RPC shape. It has one
intended behavior change, a fix ([§5](#5-the-one-behavior-change)).

It comes before write-queue plan 3 (rebasing, [write-queue design §6.4](2026-11-06-write-queue-design.md#64-rebasing-when-an-edit-merges-and-when-it-is-refused)),
which needs the planner this design extracts.

## 1. What is wrong today

Each item was found by reading the code at `2d30601`; line numbers are from that revision.

1. **Many stores for one vdir.** A `Store` is opened per session
   (`twydo-daemon/src/server/sessions.rs:186`) and again, as a throwaway, for
   `journal.revert` (`writes.rs:759`), each drain (`writes.rs:1013`), `journal.abandon`
   (`admin.rs:314`), startup recovery (`server/mod.rs:574`), a conflict resolution
   (`twydo-core/src/sync/resolve.rs:193`) and the sync runner's drain
   (`store/queue.rs:152`). All write the same journal and queue.
2. **Per-call state passed through setters.** `Store` has `set_request`, `set_vdir_busy`,
   `set_fence_at_drain`, `set_collect_drained` and `set_lock_wait`. Every daemon write sets
   three of them, calls the store and unsets them (`writes.rs:505-519`, repeated at
   `writes.rs:650-665`). The poisoned-mutex path must remember to clear the request
   (`sessions.rs:25`). Results come back through a side channel (`take_drained`).
3. **Exclusion decided twice.** The lane already says whether a long job holds the vdir.
   `Store::submit` still polls the file lock for 250 ms (`store/mod.rs:688`), and
   `settle_queued` probes it again after the push (`store/mod.rs:746`) to close a
   lost-wakeup window between processes.
4. **Undo history kept twice.** `Store.done/undone` hold the steps; `Session.done/undone`
   hold their titles, documented as "in step with the store's stacks"
   (`sessions.rs:62-66`). `prune_refused` and `prune_refused_steps` exist to resynchronize
   them, and are why a drain takes session mutexes.
5. **Planning is tied to the writer.** `prepare` and `plan*` are `Store` methods. The
   family, order and reparent planners use `self` twice in total. A drain cannot re-plan a
   queued op against changed files; it can only re-gate and replay the stored bytes
   (`store/mod.rs:846-854`).
6. **The sync runner's drain ignores the daemon's matrix.** `Store::for_drain` always uses
   `Matrix::embedded()` (`store/mod.rs:384`). A daemon started with `--matrix` gates a
   submitted edit with the custom matrix, but an edit queued during a sync and drained by
   the runner is re-gated with the embedded one. `sync/run.rs` has no matrix parameter at
   all. This follows directly from item 1: the runner cannot reach the daemon's store, so it
   builds its own.

The cost shows in the history. `store/mod.rs` is the most-changed source file since
2026-11-05 (24 commits). `writes.rs` opens with a 110-line lock-order comment. Most review
items deferred from daemon plans 1 and 2 are about this seam: the drain/undo lock order, a
revert opened outside the lane and gate, a drain re-gating with a stale configuration, the
fence flag deferring unrelated edits.

## 2. Goals

1. One object writes to the vdir, the journal and the queue. The daemon holds exactly one.
2. Planning an operation is a pure function of a snapshot, the operation, the write policy
   and the time.
3. Whatever varies per call (request, time, whether the vdir is free) is an argument or a
   type, never a field set beforehand.
4. A session's undo history is one list.
5. Every drain, whoever runs it, uses the daemon's matrix and its live configuration.

## 3. Non-goals

- The daemon's threading model. Threads, the lane, the commit gate, the publisher, the job
  runner and the scheduler stay. An actor rewrite is not part of this.
- Gate decisions, `edit` functions, the compatibility matrix.
- Durable formats: journal entry format 1, queue items, `.claim`, `cancelled/`, `refused/`,
  the collection-operation fence. A new daemon must read what an old one left.
- The RPC protocol and `schema/tdo.openrpc.json`.
- Rebasing. A drain stays strict: re-gate, compare bases whole file by whole file, refuse on
  a mismatch.
- Collection operations (`lists/`), conflicts storage and retirement, except where they call
  `Store` today.

## 4. Design

### 4.1 The planner

```rust
// twydo-core/src/store/plan.rs
pub struct Policy<'a> { pub matrix: &'a Matrix, pub mode: WriteMode }

pub fn plan(snap: &Snapshot, op: &Op, policy: Policy<'_>, now: DateTime<Utc>)
    -> Result<Prepared, WriteError>;
```

`plan` is today's `Store::prepare` minus its I/O: the gate, the per-operation plan (the
`match` in `Store::plan`, `plan_save`, and the family, order and reparent planners), and the
holds and family guards. It reads nothing from disk and takes no lock. The vdir root it needs
for relative paths comes from `snap.config`. `Prepared` keeps its serialized form, because
queue items contain it.

The collection-fence check (`lists::operation::ensure_idle`) reads the state directory, so it
leaves `prepare` and becomes the writer's job ([§4.2](#42-the-writer)).

`op.check` and the permission reports keep calling the gate directly; nothing changes for
them.

### 4.2 The writer

```rust
// twydo-core/src/store/writer.rs
pub struct Writer { /* vdir, state dir, journal, queue, matrix, mode */ }

pub struct Ctx<'a> {
    pub now: DateTime<Utc>,
    pub request: Option<RequestRef>,
    pub config: &'a Config,      // live: time zone for a drain's "today"
}

impl Writer {
    pub fn open(cfg: &Config, matrix: Matrix) -> Result<Writer, WriteError>;
    pub fn policy(&self) -> Policy<'_>;

    /// The exclusive vdir lock, waiting at most `wait`. `None`: someone outside holds it.
    pub fn lock(&self, wait: Duration) -> Result<Option<Locked<'_>>, WriteError>;
    /// For a caller that already holds the lock (the sync runner).
    pub fn with_lock<'a>(&'a self, held: &'a VdirLock) -> Locked<'a>;

    /// Without the vdir lock.
    pub fn enqueue(&self, kind: Kind, label: &str, fence: Fence, ctx: &Ctx) -> Result<Item, WriteError>;
    pub fn cancel(&self, item: &Item, ctx: &Ctx) -> Result<bool, WriteError>;
}

impl Locked<'_> {
    pub fn recover(&self) -> Result<RecoveryReport, WriteError>;
    pub fn commit(&self, p: Prepared, ctx: &Ctx) -> Result<String, WriteError>;
    pub fn revert(&self, entry: &str, ctx: &Ctx) -> Result<String, WriteError>;
    pub fn restore(&self, path: &str, expect: Option<&str>, content: Option<&str>,
                   uids: Vec<String>, ctx: &Ctx) -> Result<String, WriteError>;
    pub fn abandon(&self, entry: &str) -> Result<bool, WriteError>;
    pub fn drain(&self, ctx: &Ctx) -> Result<Vec<Drained>, WriteError>;
}

pub enum Fence { Now, AtDrain }
```

Points that matter:

- **`Writer` has no per-call state** and takes `&self`. It is `Sync`; the daemon keeps one
  in `Shared`.
- **Holding the lock is a type.** Everything that touches the vdir is a method of `Locked`.
  A commit without the lock does not compile.
- **Commit or queue is the caller's decision.** The writer no longer has a `submit` that
  picks. The daemon decides from the lane ([§4.4](#44-the-daemon)).
- **`drain` returns what it drained.** There is no collector flag. A commit that must drain
  first calls `drain` and then `commit`, and has both results in hand.
- **`Fence` replaces `set_fence_at_drain`.** `enqueue` checks the collection fence for
  `Fence::Now` and leaves it to the drain for `Fence::AtDrain`. `commit` always checks it.
- **The drain takes the live configuration** from `Ctx`, so a time-zone change is not
  half-applied (deferred from daemon plan 2, task 9).
- **`WriteError::Busy` leaves the commit path.** `lock` returning `None` is the "busy" of a
  write, and the caller queues. The variant stays for one case: an undo of a queued step
  that a drainer has claimed but not yet committed.
- **`lock_waiting`** blocks for the lock, for the paths that cannot be queued (conflict
  resolution, abandon, list recovery).

The durable steps keep their order, which is what crash recovery depends on:

| Path | Order on disk |
|---|---|
| Commit | journal entry `prepared` → file writes → stage `applied` |
| Queue | `<id>.json` written |
| Drain of one item | rename to `<id>.claim` → commit (entry names `from_queue`) → claim removed |
| Cancel | `cancelled/<id>.json` written → `<id>.json` removed (record removed again if a drainer claimed first) |
| Refusal | `refused/<id>.json` written → claim removed |

The claim rename stays. It is still a real race inside one process: an undo during a long job
cancels a queued item without the lane while the sync runner drains.

### 4.3 The history

```rust
// twydo-core/src/store/history.rs
pub struct Step { pub title: String, pub state: StepState }
pub enum StepState { Applied(String), Queued(Item), Cancelled(Item) }

pub struct History { done: Vec<Step>, undone: Vec<Step> }

impl History {
    pub fn record(&mut self, title: String, s: &Submitted);
    pub fn undo(&mut self, access: Access<'_>, origin: &str, ctx: &Ctx, drained: &mut Vec<Drained>)
        -> Result<(Submitted, Step), WriteError>;
    pub fn redo(/* the same */);
}

pub enum Access<'a> {
    Try { writer: &'a Writer, wait: Duration, fence: Fence },   // lock only if a step needs it
    Queue(&'a Writer, Fence),                                   // a long job holds the vdir
}
```

The history takes the vdir lock itself and only when a step needs it, which keeps today's
waits: none to cancel a queued step, `wait` to revert an applied one, a single probe after
re-queueing a cancelled one. Every drain it runs is appended to `drained`, also when the call
then fails, so nothing is collected in a field.

One list carries both the step and its title. `undo` and `redo` are today's `undo_last`,
`redo_last` and `revert_step`, with the same rules: a queued step is cancelled if it is still
queued, a step whose item a drain refused is skipped, a cancelled step is queued again under
a new id. The moved step is returned so the daemon can fill `undid`.

Because a refused step is skipped when it is reached, the eager pruning
(`Session::prune_refused`, `writes::prune_refused_steps`) is no longer needed to keep two
stacks aligned. That is expected to remove the session mutexes from the drain hook's lock
order; the plan confirms it against the `undid` title tests before deleting the pruning.

`Session` in the daemon becomes `{ history: History }`.

### 4.4 The daemon

`Shared` gains `writer: Writer`. One function in `writes.rs` is the only way to the
writer:

```rust
fn with_vdir<T>(shared: &Shared, lane: &impl LaneHeld,
                f: impl FnOnce(&Writer) -> T) -> Result<T, RpcError>;
```

It takes the commit gate's read side and runs `f` with the writer. `LaneHeld` is implemented
by the write guard and the long guard, so the call needs a lane guard to compile. Every path
that can commit, queue or drain goes through it: `op.submit`, `op.undo`, `op.redo`,
`journal.revert`, `journal.abandon`, `journal.recover`, `conflicts.resolve`, `lists.recover`,
the drains and startup recovery. Inside it the vdir lock is taken through the writer
(`lock(250 ms)` for a task write or a drain, lazily by the history for an undo or redo,
`lock_waiting` for the paths that cannot be queued).

A task write then reads:

| Lane | Vdir lock | What happens |
|---|---|---|
| write guard | got it | `drain`, then `plan` + `commit` (or `revert`) |
| write guard | held outside for the whole wait | `enqueue(Fence::Now)`, then `request_drain` |
| `LongBusy(Sync)` | not tried | `enqueue(Fence::Now)` |
| `LongBusy(List)` | not tried | `enqueue(Fence::AtDrain)` |

Row 2 is today's fallthrough when the 250 ms poll fails. The wait stays 250 ms. The second
probe after the push is dropped: with the write lane held no long job runs, so no other
drainer can have missed the item, and an outside holder never drains. The `LongBusy` rows
already request a drain when the job has ended meanwhile (`writes.rs:620`).

The lock-order comment shrinks accordingly: steps 3 to 5 of a task write (lane, commit gate,
vdir lock) become "the lane, then `with_vdir`, then the writer's lock".

### 4.5 The sync runner and conflict resolution

`sync::run` takes the writer as an argument (`run_with(cfg, writer, opts, now)`;
`RunOptions` stays `Default` and `Clone`, which a borrowed field would end). Its drains call
`writer.with_lock(&held).drain(ctx)`. `queue::drain_with_lock` and `Store::for_drain` are
removed.

`sync::resolve::resolve` takes `&Writer` in place of opening a store, and takes the vdir
lock itself (`Writer::lock_waiting`) at the point where it writes, as it does today: it reads
under the shared lock first. Journal recovery still runs under the lock before the write,
as before every write. It is therefore one caller of the writer's lock outside `with_vdir`; the
daemon's `conflicts.resolve` still holds the waiting lane and the commit gate around it.

`Store::recover_list` (`store/retired.rs`) becomes a function over `&Locked` and the calling
session's `History`.

### 4.6 What is removed

`Store` and its constructors (`open`, `open_with`, `without_recovery`, `for_drain`); the five
setters and `take_drained`; `submit`, `submit_with` and `settle_queued`; `Store::lock` and
`try_lock_for`; `Step` in `store/mod.rs`; `Session.store`, `Titled` and the title stacks;
`prune_refused` and `prune_refused_steps` (subject to §4.3); `queue::drain_with_lock`.

## 5. The one behavior change

A drain run by the sync runner gates with the daemon's matrix instead of the embedded one
(§1 item 6). With the default matrix nothing changes. With `tdo daemon --matrix`, an edit
queued during a sync is now admitted or refused by the same matrix as an edit applied at
once.

This is reported as a `fix(daemon)` commit of its own, with a test that fails before it: a
custom matrix that refuses an operation the embedded one admits, an edit queued behind a
sync, and the drain refusing it.

## 6. Tests

The existing suites are the contract. Nothing in `twydo-daemon/tests`, `tdo/tests` or the
process-kill tests is rewritten to fit the new types; they must pass as they are, apart from
tests that construct a `Store` directly.

- **Core tests that call `Store`** (about 180 `apply`, 140 `undo_last`/`redo_last`, 75
  `submit`, 50 `drain` call sites) keep their call shape through a wrapper in
  `tests/common`, built on `plan`, `Writer` and `History`. It is test support, not a
  production alias. Their assertions do not change.
- **Planner tests** are new and small: `plan` on a snapshot built in memory, with no
  temporary directory, for one case per planner (single task, family, order, reparent,
  save). They do not repeat `store_family`.
- **Tests of behavior that only existed between processes** are removed, each with its
  retained owner recorded in [testing.md](../../testing.md) in the form of the existing
  cleanup table. Known candidates: the `submit_with` lost-wakeup tests in `store_queue`
  (owner: the daemon's queued-path drain trigger test) and tests of `set_lock_wait`-style
  blocking (no callers left).
- **New daemon tests:** the custom-matrix drain of §5; a live time-zone change applied at a
  drain; an undo after a refused queued step answering with the right `undid` title.
- `mise run test` and `mise run lint` pass after every task.

## 7. Risks

| Risk | Guard |
|---|---|
| A crash window moves because durable steps are reordered | The order table in §4.2 is a requirement. The process-kill tests (`daemon_process`, `recovery`, `crash_after_commit`) and `journal`/`store_queue` recovery tests run unchanged |
| A queued item written by 0.3.1 fails to decode | `Prepared`, `Item`, `Entry` keep their serde form; a test decodes items and entries captured from the current build |
| Dropping the second lock probe strands an item | The argument in §4.4, plus the existing daemon test of the queued-path drain trigger; the plan adds a test for row 2 of the table |
| Removing eager pruning changes an undo message | The `undid` title tests; pruning is deleted only after they pass without it |
| The refactor grows into the daemon's threading | Non-goal. `Lane`, the commit gate and the publisher keep their interfaces |

## 8. Order of work

One plan, each task leaving the workspace green:

1. The §5 test and fix, on today's code (the runner is handed the matrix). Small, and it
   pins the behavior before anything moves.
2. Extract `plan` as a free function; `Store::prepare` calls it.
3. Add `Writer` and `Locked`; `Store` becomes a thin shell over them.
4. Add `History`; `Store` and `Session` use it.
5. Switch the daemon: `Shared.writer`, `with_vdir`, sessions hold only a history, the sync
   runner and conflict resolution take the writer.
6. Delete `Store` and the items of §4.6; move core tests to the `tests/common` wrapper.
7. Documentation: [architecture.md](../../architecture.md) (the write path, the write queue,
   locking), the lock-order comment in `writes.rs`, [testing.md](../../testing.md),
   `AGENTS.md` (this work precedes write-queue plan 3).

No version bump of its own: the refactor commits are `refactor(core)` and
`refactor(daemon)`; the §5 fix releases as a patch.

## 9. Decisions

Made by the user on 2026-11-13 (fixed):

1. **The §5 fix is the first task of this plan**, as its own `fix(daemon)` commit with its
   test, not a separate PR beforehand.
2. **The outside-lock wait stays 250 ms** (row 2 of §4.4). A write does not switch to a
   single non-blocking try.
