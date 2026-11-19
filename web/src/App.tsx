import { useEffect, useState } from "preact/hooks";
import type { DataSource } from "./data";
import type { ViewRow } from "./gen/protocol";
import { Banner, type BannerState } from "./components/Banner";
import { CaptureSheet } from "./components/CaptureSheet";
import { DetailSheet } from "./components/DetailSheet";
import { CalendarDayIcon, MailboxIcon, PlusIcon, SpinnerIcon } from "./components/Icons";
import { Offline } from "./components/Offline";
import { Pairing } from "./components/Pairing";
import { SyncWarning } from "./components/SyncWarning";
import { dayOf, TabBar, type Tab } from "./components/TabBar";
import { TaskList } from "./components/TaskList";

const TAB_KEY = "twydo.tab";

function loadTab(): Tab {
  try {
    return localStorage.getItem(TAB_KEY) === "today" ? "today" : "inbox";
  } catch {
    return "inbox";
  }
}

let bannerSeq = 0;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ ds }: { ds: DataSource }) {
  const [tab, setTab] = useState<Tab>(loadTab);
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
  const view = ds.view({ view: tab }).value;
  // Any view will do for the date; prefer the one on screen.
  const today = view?.today ?? ds.view({ view: "inbox" }).value?.today ?? ds.view({ view: "today" }).value?.today;

  useEffect(() => {
    ds.refresh({ view: tab }).catch(() => {});
  }, [tab]);

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

  const select = (t: Tab) => {
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
      const now = ds.view({ view: tab }).value;
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

  return (
    <div class="app">
      {/* A cached view shows at once and refreshes in place; only offline dims it (spec §9.4). */}
      <main class={offline ? "screen is-offline" : "screen"}>
        <header class="large-title">
          <h1>{view?.title ?? (tab === "today" ? "Today" : "Inbox")}</h1>
          {view && <span class="count">{view.open_count}</span>}
        </header>
        {view?.sync?.attention && <SyncWarning sync={view.sync} />}
        {offline && <Offline updated={updated} onRetry={() => location.reload()} />}
        {!view && !offline && (
          <div class="loading" role="status" aria-label="Loading">
            <SpinnerIcon />
          </div>
        )}
        {view && (
          <TaskList
            key={tab}
            view={view}
            done={new Set(done.keys())}
            busy={busy}
            disabled={offline}
            empty={<EmptyList tab={tab} day={dayOf(today)} canAdd={canAdd} />}
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
      {!offline && (
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

function EmptyList({ tab, day, canAdd }: { tab: Tab; day: string; canAdd: boolean }) {
  return (
    <div class="empty">
      <span class="empty-icon">
        {tab === "today" ? <CalendarDayIcon day={day} size={72} /> : <MailboxIcon size={72} />}
      </span>
      <p class="empty-title">Nothing here</p>
      {canAdd && <p class="empty-hint">Tap + to add a new task</p>}
    </div>
  );
}
