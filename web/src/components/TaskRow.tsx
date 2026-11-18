import { useLayoutEffect, useRef } from "preact/hooks";
import type { ViewRow } from "../gen/protocol";
import { AlertIcon, ChecklistIcon, NoteIcon, ProjectIcon, RepeatIcon } from "./Icons";

export function TaskRow({
  row,
  done,
  busy,
  disabled,
  motion,
  onToggle,
  onOpen,
}: {
  row: ViewRow;
  done: boolean;
  busy: boolean;
  disabled?: boolean;
  /** Animate in after a refresh added the row, or out after a refresh dropped it. */
  motion?: "entering" | "leaving";
  onToggle: () => void;
  onOpen: () => void;
}) {
  const finished = done || row.completed;
  const urgent = row.overdue || row.due_today;
  const sub = row.due ? row.due : row.start ? `Starts ${row.start}` : "";
  const li = useRef<HTMLLIElement>(null);
  // The height animations run between zero and the row's own height.
  useLayoutEffect(() => {
    if (motion && li.current) li.current.style.setProperty("--h", `${li.current.scrollHeight}px`);
  }, [motion]);
  const leaving = motion === "leaving";
  const cls = ["row", finished && "done", motion].filter(Boolean).join(" ");
  return (
    <li ref={li} class={cls} style={{ "--depth": row.depth }} aria-hidden={leaving || undefined}>
      <button
        type="button"
        role="checkbox"
        class="check"
        aria-checked={finished}
        aria-label={`Complete ${row.title}`}
        disabled={finished || busy || disabled || leaving}
        onClick={onToggle}
      />
      <button type="button" class="body" disabled={leaving} onClick={onOpen}>
        <span class="title">{row.title}</span>
        {sub && <span class={row.due && urgent ? "sub urgent" : "sub"}>{sub}</span>}
      </button>
      <span class="markers">
        {row.note && (
          <span class="marker" aria-label="Has a note">
            <NoteIcon />
          </span>
        )}
        {row.repeats && (
          <span class="marker" aria-label="Repeats">
            <RepeatIcon />
          </span>
        )}
        {row.kind === "Project" && (
          <span class="marker" aria-label="Project">
            <ProjectIcon />
          </span>
        )}
        {row.kind === "Checklist" && (
          <span class="marker" aria-label="Checklist">
            <ChecklistIcon />
          </span>
        )}
        {row.conflict && (
          <span class="marker urgent" aria-label="Conflict">
            <AlertIcon />
          </span>
        )}
        {(row.pending || busy) && <span class="pending-dot" aria-label="Waiting to sync" />}
      </span>
    </li>
  );
}
