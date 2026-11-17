import type { ViewRow } from "../gen/protocol";
import { AlertIcon, ChecklistIcon, NoteIcon, ProjectIcon, RepeatIcon } from "./Icons";

export function TaskRow({
  row,
  done,
  busy,
  disabled,
  onToggle,
  onOpen,
}: {
  row: ViewRow;
  done: boolean;
  busy: boolean;
  disabled?: boolean;
  onToggle: () => void;
  onOpen: () => void;
}) {
  const finished = done || row.completed;
  const urgent = row.overdue || row.due_today;
  const sub = row.due ? row.due : row.start ? `Starts ${row.start}` : "";
  return (
    <li class={finished ? "row done" : "row"} style={{ "--depth": row.depth }}>
      <button
        type="button"
        role="checkbox"
        class="check"
        aria-checked={finished}
        aria-label={`Complete ${row.title}`}
        disabled={finished || busy || disabled}
        onClick={onToggle}
      />
      <button type="button" class="body" onClick={onOpen}>
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
