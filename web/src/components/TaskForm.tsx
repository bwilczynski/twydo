import { useEffect, useRef, useState } from "preact/hooks";
import type { Field, ViewsListItem, ViewsListResult } from "../gen/protocol";
import type { DataSource, FormOpen } from "../data";
import { useFormState } from "../formState";
import { ConfirmSheet } from "./ConfirmSheet";

const KINDS = ["Task", "Project", "Checklist"] as const;
const CHIPS: [string, string][] = [["15m", "15m"], ["1h", "1h"], ["1d", "1d"], ["none", "none"]];
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

const listsOf = (lists?: ViewsListResult): { id: string; name: string }[] =>
  (lists?.entries ?? [])
    .filter((e): e is ViewsListItem => "view" in e && e.view.view === "list" && !!e.view.list)
    .map((e) => ({ id: e.view.list as string, name: e.name }));

/** The full-screen edit / new-task form (spec §8.3). The daemon checks and writes; this shows. */
export function TaskForm({
  ds,
  open,
  lists,
  heading,
  saveLabel,
  onSaved,
  onCancel,
}: {
  ds: DataSource;
  open: FormOpen;
  lists?: ViewsListResult;
  heading: string;
  saveLabel: string;
  onSaved: (title: string) => void;
  onCancel: () => void;
}) {
  const f = useFormState(ds, open);
  const [busy, setBusy] = useState(false);
  const [refusal, setRefusal] = useState("");
  const [confirm, setConfirm] = useState(false);
  const notes = useRef<HTMLTextAreaElement>(null);
  const answer = f.answer;
  const values = f.values;

  useEffect(() => {
    const el = notes.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = `${el.scrollHeight}px`;
    }
  }, [values?.notes, !!answer]);

  const save = () => {
    if (!f.canSave || !answer?.op || !f.basis || !values || busy) return;
    setBusy(true);
    setRefusal("");
    const form: FormOpen = { uid: open.uid, new: open.new, initial: f.initial, values };
    ds.save(answer.op, f.basis, { uid: open.uid, title: values.title, form }).then(
      () => onSaved(values.title),
      (e) => {
        setBusy(false);
        setRefusal(e instanceof Error ? e.message : String(e));
        if ((e as { data?: { kind?: string } }).data?.kind === "stale") f.recheck();
      },
    );
  };

  const preview = (field: Field) => answer?.form.previews.find((p) => p.field === field)?.text;
  const options = listsOf(lists);
  const listId = values?.list ?? "";
  const hasList = options.some((o) => o.id === listId);

  const text = (field: "due" | "start" | "repeat" | "alert") => {
    const date = field === "due" || field === "start";
    const label = cap(field);
    const disabled = field === "alert" && answer && !answer.form.alert_editable;
    const pv = preview(field);
    return (
      <div class="field" key={field}>
        <label>
          <span class="label">{label}</span>
          <input
            aria-label={label}
            value={values?.[field] ?? ""}
            disabled={!!disabled || !values}
            autocapitalize="off"
            autocomplete="off"
            onInput={(e) => f.setField(field, (e.target as HTMLInputElement).value)}
          />
        </label>
        {date && (
          <span class="picker">
            <button
              type="button"
              class="cal"
              aria-label={`Pick ${label} date`}
              onClick={(e) => {
                const input = (e.currentTarget as HTMLElement).nextElementSibling as HTMLInputElement;
                if (input.showPicker) input.showPicker();
                else input.click();
              }}
            >
              &#128197;
            </button>
            <input
              type="date"
              class="hidden-date"
              tabIndex={-1}
              aria-label={`${label} date`}
              onChange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                if (v) f.setField(field, v);
              }}
            />
          </span>
        )}
        {field === "alert" && (
          <div class="chips">
            {CHIPS.map(([name, v]) => (
              <button type="button" class="chip" key={name} disabled={!!disabled} onClick={() => f.setField("alert", v)}>
                {name}
              </button>
            ))}
          </div>
        )}
        {pv && <p class="sub preview">{pv}</p>}
      </div>
    );
  };

  const render = (field: Field) => {
    if (!values) return null;
    switch (field) {
      case "title":
        return (
          <input
            key={field}
            aria-label="Title"
            placeholder="Title"
            value={values.title}
            onInput={(e) => f.setField("title", (e.target as HTMLInputElement).value)}
          />
        );
      case "list":
        return (
          <label class="field" key={field}>
            <span class="label">List</span>
            <select aria-label="List" value={listId} onChange={(e) => f.setField("list", (e.target as HTMLSelectElement).value)}>
              {!hasList && <option value={listId}>{listId}</option>}
              {options.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.name}
                </option>
              ))}
            </select>
          </label>
        );
      case "type":
        return (
          <div class="field" key={field}>
            <span class="label">Type</span>
            <div class="segmented" role="radiogroup" aria-label="Type">
              {KINDS.map((k) => (
                <button
                  type="button"
                  role="radio"
                  key={k}
                  aria-checked={values.kind === k}
                  class={values.kind === k ? "on" : ""}
                  onClick={() => f.setField("type", k)}
                >
                  {k}
                </button>
              ))}
            </div>
          </div>
        );
      case "notes":
        return (
          <textarea
            key={field}
            ref={notes}
            aria-label="Notes"
            placeholder="Notes"
            rows={3}
            value={values.notes}
            onInput={(e) => f.setField("notes", (e.target as HTMLTextAreaElement).value)}
          />
        );
      default:
        return text(field);
    }
  };

  const showError = f.typed && answer?.error && answer.error !== "no changes";
  return (
    <>
      <form
        class="sheet full task-form"
        aria-label={heading}
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <header>
          <button type="button" onClick={() => (f.dirty ? setConfirm(true) : onCancel())}>
            Cancel
          </button>
          <h2>{heading}</h2>
          <button type="submit" class="strong" disabled={!f.canSave || busy}>
            {saveLabel}
          </button>
        </header>
        {f.changed && (
          <p class="notice" role="status">
            Changed since you opened it{" "}
            <button type="button" disabled={f.checking} onClick={f.review}>
              Review
            </button>
          </p>
        )}
        {f.reviewed && f.reviewed.overwritten.length > 0 && (
          <p class="sub">Your typing replaces their change to: {f.reviewed.overwritten.join(", ")}</p>
        )}
        {f.reviewed && f.reviewed.dropped.length > 0 && (
          <p class="sub">Can no longer be set: {f.reviewed.dropped.join(", ")}</p>
        )}
        {showError && (
          <p class="urgent" role="alert">
            {answer!.error}
          </p>
        )}
        {f.failed && (
          <p class="urgent" role="alert">
            Couldn't check the form{" "}
            <button type="button" onClick={f.recheck}>
              Retry
            </button>
          </p>
        )}
        {refusal && (
          <p class="urgent" role="alert">
            {refusal}
          </p>
        )}
        {(answer?.form.fields ?? []).map(render)}
      </form>
      {confirm && (
        <ConfirmSheet
          title="Discard changes?"
          confirmLabel="Discard"
          cancelLabel="Keep editing"
          destructive
          onConfirm={onCancel}
          onCancel={() => setConfirm(false)}
        />
      )}
    </>
  );
}
