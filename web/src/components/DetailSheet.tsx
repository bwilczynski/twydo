import type { ViewRow } from "../gen/protocol";

export function DetailSheet({ row, onClose }: { row: ViewRow; onClose: () => void }) {
  return (
    <div class="scrim" onClick={onClose}>
      <div class="sheet" role="dialog" aria-label={row.title} onClick={(e) => e.stopPropagation()}>
        <header>
          <span />
          <button type="button" onClick={onClose}>
            Done
          </button>
        </header>
        <h2>{row.title}</h2>
        <dl>
          {row.due && (
            <>
              <dt>Due</dt>
              <dd>{row.due}</dd>
            </>
          )}
          {row.start && (
            <>
              <dt>Starts</dt>
              <dd>{row.start}</dd>
            </>
          )}
          <dt>List</dt>
          <dd>{row.list.name}</dd>
          {row.parent && (
            <>
              <dt>In</dt>
              <dd>{row.parent.title}</dd>
            </>
          )}
        </dl>
        {row.note && <p class="note">{row.note}</p>}
        {row.conflict && <p class="urgent">This task has a sync conflict. Resolve it in tdo.</p>}
      </div>
    </div>
  );
}
