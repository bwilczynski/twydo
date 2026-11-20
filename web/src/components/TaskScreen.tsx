import { Fragment } from "preact";
import type { ChildRow, TasksGetResult } from "../gen/protocol";
import { ChecklistIcon, ChevronLeftIcon, NoteIcon, ProjectIcon } from "./Icons";

const URL_RE = /(https?:\/\/[^\s<>]+)/g;

/** A link without the sentence punctuation after it; a `)` stays when it closes a `(` in the link. */
function trimLink(part: string): string {
  let url = part.replace(/[.,;:!?\]]+$/, "");
  while (url.endsWith(")") && url.split(")").length > url.split("(").length) {
    url = url.slice(0, -1).replace(/[.,;:!?\]]+$/, "");
  }
  return url;
}

/** The note with its links made tappable; line breaks are kept by the stylesheet. */
function Linked({ text }: { text: string }) {
  return (
    <>
      {text.split(URL_RE).map((part, i) => {
        if (i % 2 === 0) return part;
        // Sentence punctuation after a link is not part of it.
        const url = trimLink(part);
        return (
          <Fragment key={i}>
            <a href={url} target="_blank" rel="noopener">
              {url}
            </a>
            {part.slice(url.length)}
          </Fragment>
        );
      })}
    </>
  );
}

function Check({
  title,
  done,
  disabled,
  onClick,
}: {
  title: string;
  done: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      class="check"
      aria-checked={done}
      aria-label={`${done ? "Reopen" : "Complete"} ${title}`}
      disabled={disabled}
      onClick={onClick}
    />
  );
}

export function TaskScreen({
  result,
  ticked,
  busy,
  disabled,
  backLabel,
  onBack,
  onToggle,
  onOpen,
  onEdit,
  onAddItem,
  onDelete,
}: {
  result: TasksGetResult;
  /** Target state shown for a task while its write is in flight (true = done). */
  ticked: Map<string, boolean>;
  busy: Set<string>;
  disabled: boolean;
  backLabel: string;
  onBack: () => void;
  /** `done` is the state the tap asks for: true completes, false reopens. */
  onToggle: (uid: string, title: string, done: boolean) => void;
  onOpen: (uid: string) => void;
  onEdit: () => void;
  onAddItem: () => void;
  onDelete: () => void;
}) {
  const { task, children } = result;
  const finished = (uid: string, completed: boolean) => ticked.get(uid) ?? completed;
  const done = finished(task.uid, task.completed);
  const open = children.filter((c) => !finished(c.uid, c.completed)).length;
  const addable = task.kind === "Project" || task.kind === "Checklist";
  const kind = task.kind === "Task" ? "" : `${task.kind} · `;
  return (
    <>
      <div class="task-bar">
        <button type="button" class="back" onClick={onBack}>
          <ChevronLeftIcon />
          {backLabel}
        </button>
        <button type="button" class="edit" disabled={disabled} onClick={onEdit}>
          Edit
        </button>
      </div>
      <header class={done ? "task-head done" : "task-head"}>
        <Check
          title={task.title}
          done={done}
          disabled={disabled || busy.has(task.uid)}
          onClick={() => onToggle(task.uid, task.title, !done)}
        />
        <h1>{task.title}</h1>
        {(task.pending || busy.has(task.uid)) && <span class="pending-dot" aria-label="Waiting to sync" />}
      </header>
      <p class="task-meta">
        {kind}
        {task.list.color && <span class="list-dot" style={{ background: task.list.color }} />}
        {task.list.name}
      </p>
      <dl class="facts">
        {task.due && (
          <>
            <dt>Due</dt>
            <dd class={task.overdue || task.due_today ? "urgent" : undefined}>{task.due}</dd>
          </>
        )}
        {task.repeat && (
          <>
            <dt>Repeat</dt>
            <dd>{task.repeat}</dd>
          </>
        )}
        {task.alerts.map((a, i) => (
          <Fragment key={i}>
            {i === 0 && <dt>Alert</dt>}
            <dd>{a}</dd>
          </Fragment>
        ))}
        {task.start && (
          <>
            <dt>Start</dt>
            <dd>{task.start}</dd>
          </>
        )}
        {task.parent && (
          <>
            <dt>In</dt>
            <dd>
              <button type="button" class="link" onClick={() => onOpen(task.parent!.uid)}>
                {task.parent.title}
              </button>
            </dd>
          </>
        )}
      </dl>
      {task.warnings.map((w, i) => (
        <p key={i} class="urgent">
          ! {w}
        </p>
      ))}
      {task.blocked && <p class="urgent">! {task.blocked}</p>}
      {task.conflict && <p class="urgent">This task has a sync conflict. Resolve it in tdo.</p>}
      {(children.length > 0 || addable) && (
        <section class="group">
          <h2>Items · {open} open</h2>
          <ul class="card">
            {children.map((c) => (
              <ItemRow
                key={c.uid}
                child={c}
                done={finished(c.uid, c.completed)}
                busy={busy.has(c.uid)}
                disabled={disabled}
                onToggle={(d) => onToggle(c.uid, c.title, d)}
                onOpen={() => onOpen(c.uid)}
              />
            ))}
            {addable && (
              <li class="row add-item">
                <button type="button" class="body" disabled={disabled} onClick={onAddItem}>
                  + Add item
                </button>
              </li>
            )}
          </ul>
        </section>
      )}
      {task.note && (
        <section class="group">
          <h2>Notes</h2>
          <div class="card note-card">
            <p class="note">
              <Linked text={task.note} />
            </p>
          </div>
        </section>
      )}
      <button type="button" class="delete-task" disabled={disabled} onClick={onDelete}>
        Delete task
      </button>
    </>
  );
}

function ItemRow({
  child,
  done,
  busy,
  disabled,
  onToggle,
  onOpen,
}: {
  child: ChildRow;
  done: boolean;
  busy: boolean;
  disabled: boolean;
  onToggle: (done: boolean) => void;
  onOpen: () => void;
}) {
  const urgent = child.overdue || child.due_today;
  return (
    <li class={done ? "row done" : "row"}>
      <Check title={child.title} done={done} disabled={disabled || busy} onClick={() => onToggle(!done)} />
      <button type="button" class="body" onClick={onOpen}>
        <span class="title">{child.title}</span>
        {child.due && <span class={urgent ? "sub urgent" : "sub"}>{child.due}</span>}
      </button>
      <span class="markers">
        {child.has_note && (
          <span class="marker" aria-label="Has a note">
            <NoteIcon />
          </span>
        )}
        {child.kind === "Project" && (
          <span class="marker" aria-label="Project">
            <ProjectIcon />
          </span>
        )}
        {child.kind === "Checklist" && (
          <span class="marker" aria-label="Checklist">
            <ChecklistIcon />
          </span>
        )}
        {(child.pending || busy) && <span class="pending-dot" aria-label="Waiting to sync" />}
      </span>
    </li>
  );
}
