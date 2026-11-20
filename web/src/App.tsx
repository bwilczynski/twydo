import { useEffect, useRef, useState } from "preact/hooks";
import { viewKey, type DataSource, type ViewRef, type Visible } from "./data";
import { useTaskStack } from "./taskStack";
import type { ViewRow, ViewsGetResult, ViewsListItem, ViewsListResult } from "./gen/protocol";
import { Banner, type BannerState } from "./components/Banner";
import { CaptureSheet } from "./components/CaptureSheet";
import { CalendarDayIcon, ChevronLeftIcon, ListIcon, MailboxIcon, PlusIcon, SpinnerIcon } from "./components/Icons";
import { Offline } from "./components/Offline";
import { Pairing } from "./components/Pairing";
import { SyncWarning } from "./components/SyncWarning";
import { dayOf, TabBar, type Tab } from "./components/TabBar";
import { TaskList } from "./components/TaskList";
import { TaskScreen } from "./components/TaskScreen";
import { ViewsScreen } from "./components/ViewsScreen";

const TAB_KEY = "twydo.tab";
const OPEN_KEY = "twydo.open";
const VIEWS: string[] = ["inbox", "today", "all", "scheduled", "done", "next_3_days", "list"];

function loadTab(): Tab {
  try {
    const t = localStorage.getItem(TAB_KEY);
    return t === "today" || t === "views" ? t : "inbox";
  } catch {
    return "inbox";
  }
}

/** The view open in the Views tab; malformed or missing storage means none. */
function loadOpen(): ViewRef | undefined {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    const v = raw ? JSON.parse(raw) : undefined;
    if (!v || typeof v !== "object" || typeof v.view !== "string" || !VIEWS.includes(v.view)) return undefined;
    // A list names its collection; no other view does.
    return (v.view === "list") === (typeof v.list === "string") ? v : undefined;
  } catch {
    return undefined;
  }
}

function saveOpen(ref: ViewRef | undefined) {
  try {
    if (ref) localStorage.setItem(OPEN_KEY, JSON.stringify(ref));
    else localStorage.removeItem(OPEN_KEY);
  } catch {
    // storage unavailable: the open view just is not remembered
  }
}

/** Which empty-state art a view gets. */
function emptyKind(ref: ViewRef | undefined): "inbox" | "today" | "other" {
  return ref?.view === "today" || ref?.view === "inbox" ? ref.view : "other";
}

/** The heading: the view's own title, else its Views-list name, else a blank that keeps the height. */
function titleOf(ref: ViewRef | undefined, view: ViewsGetResult | undefined, list: ViewsListResult | undefined) {
  if (view) return view.title;
  if (ref?.view === "today") return "Today";
  if (ref?.view === "inbox") return "Inbox";
  const entry = ref && list?.entries.find((e): e is ViewsListItem => "view" in e && viewKey(e.view) === viewKey(ref));
  return entry?.name ?? "\u00a0";
}

type Tick = { target: boolean; rev: number | null };
const without = (m: Map<string, Tick>, uid: string) => {
  if (!m.has(uid)) return m;
  const n = new Map(m);
  n.delete(uid);
  return n;
};

let bannerSeq = 0;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ ds }: { ds: DataSource }) {
  const [tab, setTab] = useState<Tab>(loadTab);
  // The view pushed inside the Views tab; none on the list screen.
  const [open, setOpen] = useState<ViewRef | undefined>(loadOpen);
  const openRef = useRef(open);
  openRef.current = open;
  // What is on screen once every task screen is popped.
  const rootRef = useRef<Visible>("views");
  // Rows and tasks ticked locally: the state asked for, and the revision of the write's answer
  // (null while in flight). Once a result that new arrives, the item's own state is trusted.
  const [ticks, setTicks] = useState<Map<string, Tick>>(new Map());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [writes, setWrites] = useState(0);
  const [banner, setBanner] = useState<BannerState>();
  const tasks = useTaskStack();
  const top = tasks.stack.at(-1);
  // The capture sheet, open or not; `initial` restores the text of a refused capture.
  const [capturing, setCapturing] = useState<{ initial?: { title: string; note: string } }>();

  const connection = ds.connection.value;
  // The view on screen: a tab's own, or the one opened in the Views tab (none on the list).
  const ref: ViewRef | undefined = tab === "views" ? open : { view: tab };
  rootRef.current = ref ?? "views";
  const visible: Visible = top ? { task: top } : (ref ?? "views");
  const view = ref ? ds.view(ref).value : undefined;
  const viewsList = ds.viewsList.value;
  // Any fetch will do for the date; prefer what is on screen: the view, or the list screen.
  const today =
    (tab === "views" && !open ? viewsList?.today : view?.today) ??
    ds.view({ view: "inbox" }).value?.today ??
    ds.view({ view: "today" }).value?.today ??
    viewsList?.today;

  // A list that is gone: back to the list screen, with a word about it. Subscribed before the
  // refresh below, so a remembered list that is gone is caught on the first fetch.
  useEffect(
    () =>
      ds.onGone((gone) => {
        const o = openRef.current;
        if (!o || viewKey(o) !== viewKey(gone)) return;
        openRef.current = undefined;
        setOpen(undefined);
        saveOpen(undefined);
        setBanner({ id: ++bannerSeq, text: "That list is gone", undo: false });
      }),
    [ds],
  );

  // A task that is gone: one screen back, with a word about it. The visible target stays on that
  // task until it is popped, so pop and retarget here, not on the next render.
  useEffect(
    () =>
      ds.onTaskGone((uid) => {
        const s = tasks.current();
        if (s.at(-1) !== uid) return;
        tasks.pop();
        const prev = s.at(-2);
        ds.refresh(prev ? { task: prev } : rootRef.current).catch(() => {});
        setBanner({ id: ++bannerSeq, text: "That task is gone", undo: false });
      }),
    [ds],
  );

  const visibleKey = typeof visible === "string" ? visible : "task" in visible ? `task:${visible.task}` : viewKey(visible);
  useEffect(() => {
    ds.refresh(visible).catch(() => {});
  }, [visibleKey]);

  // Ticks the result on screen has caught up with are dropped; so are rows a view no longer has.
  const shown = top ? ds.task(top).value : undefined;
  useEffect(() => {
    const seen = top ? shown : view;
    if (!seen) return;
    const live = top ? undefined : new Set(view!.sections.flatMap((s) => s.rows.map((r) => r.uid)));
    setTicks((m) => {
      const next = new Map(
        [...m].filter(([u, t]) => (!live || live.has(u)) && (t.rev === null || seen.revision < t.rev)),
      );
      return next.size === m.size ? m : next;
    });
  }, [view, shown, top]);

  // A queued write the daemon later refused: untick the row and say why.
  useEffect(
    () =>
      ds.onRefusal((r) => {
        if (r.kind === "complete") {
          setTicks((t) => without(t, r.uid));
          setBanner({ id: ++bannerSeq, text: r.message, undo: false });
        } else if (r.kind === "create") {
          const initial = { title: r.title, note: r.note };
          setBanner({
            id: ++bannerSeq,
            text: `Could not add "${r.title}": ${r.message}`,
            undo: false,
            action: { label: "Edit", run: () => { setCapturing({ initial }); setBanner(undefined); } },
          });
        } else if (r.kind === "reopen") {
          setTicks((t) => without(t, r.uid));
          setBanner({ id: ++bannerSeq, text: `Could not reopen: ${r.message}`, undo: false });
        } else {
          setBanner({ id: ++bannerSeq, text: `Could not undo: ${r.message}`, undo: false });
        }
      }),
    [ds],
  );

  if (connection === "unpaired") return <Pairing />;

  const show = (text: string, undo: boolean) => setBanner({ id: ++bannerSeq, text, undo });
  const track = async <T,>(f: () => Promise<T>): Promise<T> => {
    setWrites((n) => n + 1);
    try {
      return await f();
    } finally {
      setWrites((n) => n - 1);
    }
  };

  const openView = (r: ViewRef | undefined) => {
    setOpen(r);
    saveOpen(r);
  };

  const select = (t: Tab) => {
    // Tapping the Views tab again pops back to the list (iOS convention).
    // Any tab tap leaves the task screens; the active tab's own root shows.
    if (tasks.current().length > 0) tasks.clear();
    if (t === "views" && tab === "views") openView(undefined);
    setTab(t);
    try {
      localStorage.setItem(TAB_KEY, t);
    } catch {
      // storage unavailable: the tab just is not remembered
    }
  };

  /** Tick or untick: complete when `target` is true, else reopen. */
  async function toggle(uid: string, title: string, target: boolean) {
    setTicks((m) => new Map(m).set(uid, { target, rev: null }));
    setBusy((b) => new Set(b).add(uid));
    try {
      const r = await track(() => (target ? ds.complete(uid) : ds.reopen(uid)));
      const t = tasks.current().at(-1);
      const now = t ? ds.task(t).value : ref ? ds.view(ref).value : undefined;
      setTicks((m) => {
        if (!m.has(uid)) return m;
        // The answer's result may already have arrived.
        if (now && now.revision >= r.revision) return without(m, uid);
        return new Map(m).set(uid, { target, rev: r.revision });
      });
      show(`${target ? "Completed" : "Reopened"} "${title}"`, true);
    } catch (e) {
      setTicks((m) => without(m, uid));
      show(message(e), false);
    } finally {
      setBusy((b) => {
        const n = new Set(b);
        n.delete(uid);
        return n;
      });
    }
  }

  async function add(title: string, note: string) {
    const target = view?.capture;
    if (!target) throw new Error("There is no list to add to");
    await track(() => ds.create(title, note, target));
    setCapturing(undefined);
    show(`Added "${title}" to ${target.name}`, true);
  }

  async function undo() {
    try {
      await track(() => ds.undo());
      setTicks(new Map());
      setBanner(undefined);
    } catch (e) {
      show(message(e), false);
    }
  }

  const offline = connection === "closed";
  // Offline, a task never fetched has nothing to show: opening it would be a bare back button.
  const openTask = (uid: string) => {
    if (offline && !ds.task(uid).value) return;
    tasks.open(uid);
  };
  const updated = view?.now.slice(11, 16);
  const canAdd = !offline && !!view?.capture;
  const onList = tab === "views" && !open && !top;
  const empty = emptyKind(ref);

  const viewTitle = titleOf(ref, view, viewsList).trim() || "Views";
  const prevTitle = tasks.stack.length > 1 ? ds.task(tasks.stack.at(-2)!).value?.task.title : undefined;
  const backLabel = prevTitle ?? viewTitle;

  return (
    <div class="app">
      {/* A cached view shows at once and refreshes in place; only offline dims it (spec §9.4). */}
      <main class={offline ? "screen is-offline" : "screen"}>
        {top && shown && (
          <TaskScreen
            key={top}
            result={shown}
            ticked={new Map([...ticks].map(([u, t]) => [u, t.target]))}
            busy={busy}
            disabled={offline}
            backLabel={backLabel}
            onBack={tasks.pop}
            onToggle={toggle}
            onOpen={openTask}
          />
        )}
        {top && !shown && (
          <button type="button" class="back" onClick={tasks.pop}>
            <ChevronLeftIcon />
            {backLabel}
          </button>
        )}
        {!top && tab === "views" && open && (
          <button type="button" class="back" onClick={() => openView(undefined)}>
            <ChevronLeftIcon />
            Views
          </button>
        )}
        {!top && (
          <header class="large-title">
            <h1>{onList ? "Views" : titleOf(ref, view, viewsList)}</h1>
            {view && <span class="count">{view.open_count}</span>}
          </header>
        )}
        {!top && onList && viewsList && <ViewsScreen list={viewsList} day={dayOf(today)} onOpen={(r) => openView(r)} />}
        {!top && !onList && view?.sync?.attention && <SyncWarning sync={view.sync} />}
        {offline && <Offline updated={updated} onRetry={() => location.reload()} />}
        {(top ? !shown : onList ? !viewsList : !view) && !offline && (
          <div class="loading" role="status" aria-label="Loading">
            <SpinnerIcon />
          </div>
        )}
        {!top && view && (
          <TaskList
            key={visibleKey}
            view={view}
            done={new Set([...ticks].filter(([, t]) => t.target).map(([u]) => u))}
            busy={busy}
            disabled={offline}
            empty={<EmptyList kind={empty} day={dayOf(today)} canAdd={canAdd} />}
            onToggle={(row) => toggle(row.uid, row.title, true)}
            onOpen={(row) => openTask(row.uid)}
          />
        )}
      </main>
      {banner && (
        <Banner
          banner={banner}
          undoDisabled={writes > 0}
          onUndo={undo}
          onDismiss={() => setBanner((b) => (b?.id === banner.id ? undefined : b))}
        />
      )}
      {!offline && !onList && !top && (
        <button
          type="button"
          class="fab"
          aria-label="New task"
          disabled={!view?.capture}
          onClick={() => setCapturing({})}>
          <PlusIcon />
        </button>
      )}
      <TabBar tab={tab} day={dayOf(today)} onSelect={select} />
      {capturing && (
        <CaptureSheet initial={capturing.initial} onAdd={add} onCancel={() => setCapturing(undefined)} />
      )}
    </div>
  );
}

function EmptyList({ kind, day, canAdd }: { kind: "inbox" | "today" | "other"; day: string; canAdd: boolean }) {
  return (
    <div class="empty">
      <span class="empty-icon">
        {kind === "today" ? (
          <CalendarDayIcon day={day} size={72} />
        ) : kind === "inbox" ? (
          <MailboxIcon size={72} />
        ) : (
          <ListIcon size={72} />
        )}
      </span>
      <p class="empty-title">Nothing here</p>
      {canAdd && <p class="empty-hint">Tap + to add a new task</p>}
    </div>
  );
}
