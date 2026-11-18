import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { ViewsGetResult, ViewRow } from "../gen/protocol";
import { arrivals, departures, display, type Leaving } from "../motion";
import { TaskRow } from "./TaskRow";

/** How long a row that left the view stays on screen to animate out; matches the CSS. */
export const LEAVE_MS = 300;

export function TaskList({
  view,
  done,
  busy,
  disabled,
  empty,
  onToggle,
  onOpen,
}: {
  view: ViewsGetResult;
  done: Set<string>;
  busy: Set<string>;
  disabled?: boolean;
  /** Shown when the view has no rows (and none is still leaving). */
  empty: ComponentChildren;
  onToggle: (row: ViewRow) => void;
  onOpen: (row: ViewRow) => void;
}) {
  // Diffed against the previous view during render, so a leaving row never blinks out first.
  // A list is mounted per tab: the first view it shows has nothing to animate.
  const last = useRef<{ view?: ViewsGetResult; leaving: Leaving[]; entering: Set<string> }>({
    leaving: [],
    entering: new Set(),
  });
  const [, expire] = useState(0);
  const st = last.current;
  if (st.view !== view) {
    const fresh = st.view ? departures(st.view.sections, view.sections, done) : [];
    const live = new Set(view.sections.flatMap((s) => s.rows.map((r) => r.uid)));
    const gone = new Set(fresh.map((l) => l.row.uid));
    // Rows still leaving from an earlier change keep going unless they came back.
    const older = st.leaving.filter((l) => !live.has(l.row.uid) && !gone.has(l.row.uid));
    last.current = {
      view,
      leaving: [...older, ...fresh],
      entering: arrivals(st.view?.sections, view.sections),
    };
  }
  const { leaving, entering } = last.current;

  useEffect(() => {
    if (leaving.length === 0) return;
    const t = setTimeout(() => {
      last.current = { ...last.current, leaving: [] };
      expire((n) => n + 1);
    }, LEAVE_MS);
    return () => clearTimeout(t);
  }, [view]);

  const sections = display(view.sections, leaving);
  if (sections.length === 0) return <>{empty}</>;
  return (
    <>
      {sections.map((s) => (
        <section key={s.group} class="group">
          {s.label && <h2>{s.label}</h2>}
          <ul class="card">
            {s.rows.map(({ row: r, leaving: l }) => (
              <TaskRow
                key={r.uid}
                row={r}
                done={l ? l.done : done.has(r.uid)}
                busy={!l && busy.has(r.uid)}
                disabled={disabled}
                motion={l ? "leaving" : entering.has(r.uid) ? "entering" : undefined}
                onToggle={() => onToggle(r)}
                onOpen={() => onOpen(r)}
              />
            ))}
          </ul>
        </section>
      ))}
    </>
  );
}
