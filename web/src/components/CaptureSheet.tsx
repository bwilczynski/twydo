import { useEffect, useRef, useState } from "preact/hooks";

export function CaptureSheet({
  onAdd,
  onCancel,
  initial,
}: {
  /** Text to start with, e.g. a capture the daemon refused after the sheet had closed. */
  initial?: { title: string; note: string };
  /** Resolves when the write succeeded; rejects with the daemon's message otherwise. */
  onAdd: (title: string, note: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [title, setTitle] = useState(initial?.title ?? "");
  const [note, setNote] = useState(initial?.note ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  const ok = title.trim() !== "";
  return (
    <div class="scrim" onClick={onCancel}>
      <form
        class="sheet"
        aria-label="New task"
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          if (!ok || busy) return;
          setBusy(true);
          setError("");
          onAdd(title.trim(), note).catch((err) => {
            setBusy(false);
            setError(err instanceof Error ? err.message : String(err));
          });
        }}
      >
        <header>
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" class="strong" disabled={!ok || busy}>
            Add
          </button>
        </header>
        <input
          ref={ref}
          aria-label="Title"
          placeholder="Title"
          value={title}
          autofocus
          onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
        />
        {error && (
          <p class="urgent" role="alert">
            {error}
          </p>
        )}
        <textarea
          aria-label="Note"
          placeholder="Note"
          rows={4}
          value={note}
          onInput={(e) => setNote((e.target as HTMLTextAreaElement).value)}
        />
      </form>
    </div>
  );
}
