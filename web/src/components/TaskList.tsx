import type { ViewsGetResult, ViewRow } from "../gen/protocol";
import { TaskRow } from "./TaskRow";

export function TaskList({
  view,
  done,
  busy,
  disabled,
  onToggle,
  onOpen,
}: {
  view: ViewsGetResult;
  done: Set<string>;
  busy: Set<string>;
  disabled?: boolean;
  onToggle: (row: ViewRow) => void;
  onOpen: (row: ViewRow) => void;
}) {
  const sections = view.sections.filter((s) => s.rows.length > 0);
  if (sections.length === 0) return <p class="empty">Nothing here</p>;
  return (
    <>
      {sections.map((s, i) => (
        <section key={`${s.group}-${i}`} class="group">
          {s.label && <h2>{s.label}</h2>}
          <ul class="card">
            {s.rows.map((r) => (
              <TaskRow
                key={r.uid}
                row={r}
                done={done.has(r.uid)}
                busy={busy.has(r.uid)}
                disabled={disabled}
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
