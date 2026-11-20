import { useEffect, useRef, useState } from "preact/hooks";
import { viewKey, type DataSource, type ViewRef, type Visible } from "./data";
import type { ViewRow, ViewsGetResult, ViewsListItem, ViewsListResult } from "./gen/protocol";
import { Banner, type BannerState } from "./components/Banner";
import { CaptureSheet } from "./components/CaptureSheet";
import { DetailSheet } from "./components/DetailSheet";
import { CalendarDayIcon, ChevronLeftIcon, ListIcon, MailboxIcon, PlusIcon, SpinnerIcon } from "./components/Icons";
import { Offline } from "./components/Offline";
import { Pairing } from "./components/Pairing";
import { SyncWarning } from "./components/SyncWarning";
import { dayOf, TabBar, type Tab } from "./components/TabBar";
import { TaskList } from "./components/TaskList";
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

let bannerSeq = 0;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ ds }: { ds: DataSource }) {
  const [tab, setTab] = useState<Tab>(loadTab);
  // The view pushed inside the Views tab; none on the list screen.
  const [open, setOpen] = useState<ViewRef | undefined>(loadOpen);
  const openRef = useRef(open);
  openRef.current = open;
  // Rows ticked locally. The value is the revision of the write's answer (null while it is in
  // flight): once a view that new arrives, the row's own `completed`/`pending` is trusted.
  const [done, setDone] = useState<Map<string, number | null>>(new Map());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [writes, setWrites] = useState(0);
  const [banner, setBanner] = useState<BannerState>();
  const [detail, setDetail] = useState<ViewRow>();
  // The capture sheet, open or not; `initial` restores the text of a refused capture.
  const [capturing, setCapturing] = useState<{ initial?: { title: string; note: string } }>();

  const connection = ds.connection.value;
  // The view on screen: a tab's own, or the one opened in the Views tab (none on the list).
  const ref: ViewRef | undefined = tab === "views" ? open : { view: tab };
  const visible: Visible = ref ?? "views";
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

  const visibleKey = typeof visible === "string" ? visible : viewKey(visible);
  useEffect(() => {
    ds.refresh(visible).catch(() => {});
  }, [visibleKey]);

  // A row the view no longer has is gone for good; forget its local state.
  useEffect(() => {
    if (!view) return;
    const live = new Set(view.sections.flatMap((s) => s.rows.map((r) => r.uid)));
    setDone((d) => {
      const next = new Map([...d].filter(([u, rev]) => live.has(u) && (rev === null || view.revision < rev)));
      return next.size === d.size ? d : next;
    });
  }, [view]);

  // A queued write the daemon later refused: untick the row and say why.
  useEffect(
    () =>
      ds.onRefusal((r) => {
        if (r.kind === "complete") {
          const { uid } = r;
          setDone((d) => {
            if (!d.has(uid)) return d;
            const n = new Map(d);
            n.delete(uid);
            return n;
          });
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
    if (t === "views" && tab === "views") openView(undefined);
    setTab(t);
    try {
      localStorage.setItem(TAB_KEY, t);
    } catch {
      // storage unavailable: the tab just is not remembered
    }
  };

  async function complete(row: ViewRow) {
    setDone((d) => new Map(d).set(row.uid, null));
    setBusy((b) => new Set(b).add(row.uid));
    try {
      const r = await track(() => ds.complete(row.uid));
      const now = ref ? ds.view(ref).value : undefined;
      setDone((d) => {
        if (!d.has(row.uid)) return d;
        const n = new Map(d);
        // The answer's view may already have arrived.
        if (now && now.revision >= r.revision) n.delete(row.uid);
        else n.set(row.uid, r.revision);
        return n;
      });
      show(`Completed "${row.title}"`, true);
    } catch (e) {
      setDone((d) => {
        const n = new Map(d);
        n.delete(row.uid);
        return n;
      });
      show(message(e), false);
    } finally {
      setBusy((b) => {
        const n = new Set(b);
        n.delete(row.uid);
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
      setDone(new Map());
      setBanner(undefined);
    } catch (e) {
      show(message(e), false);
    }
  }

  const offline = connection === "closed";
  const updated = view?.now.slice(11, 16);
  const canAdd = !offline && !!view?.capture;
  const onList = tab === "views" && !open;
  const empty = emptyKind(ref);

  return (
    <div class="app">
      {/* A cached view shows at once and refreshes in place; only offline dims it (spec §9.4). */}
      <main class={offline ? "screen is-offline" : "screen"}>
        {tab === "views" && open && (
          <button type="button" class="back" onClick={() => openView(undefined)}>
            <ChevronLeftIcon />
            Views
          </button>
        )}
        <header class="large-title">
          <h1>{onList ? "Views" : titleOf(ref, view, viewsList)}</h1>
          {view && <span class="count">{view.open_count}</span>}
        </header>
        {onList && viewsList && <ViewsScreen list={viewsList} day={dayOf(today)} onOpen={(r) => openView(r)} />}
        {!onList && view?.sync?.attention && <SyncWarning sync={view.sync} />}
        {offline && <Offline updated={updated} onRetry={() => location.reload()} />}
        {(onList ? !viewsList : !view) && !offline && (
          <div class="loading" role="status" aria-label="Loading">
            <SpinnerIcon />
          </div>
        )}
        {view && (
          <TaskList
            key={visibleKey}
            view={view}
            done={new Set(done.keys())}
            busy={busy}
            disabled={offline}
            empty={<EmptyList kind={empty} day={dayOf(today)} canAdd={canAdd} />}
            onToggle={complete}
            onOpen={setDetail}
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
      {!offline && !onList && (
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
      {detail && <DetailSheet row={detail} onClose={() => setDetail(undefined)} />}
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
