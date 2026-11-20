// Synthetic form values and daemon answers shared by the form tests.
import type { FormValues, TasksFormResult } from "./gen/protocol";

export const vals = (extra: Partial<FormValues> = {}): FormValues => ({
  title: "Water plants",
  list: "c1",
  kind: "Task",
  due: "",
  start: "",
  repeat: "",
  alert: "",
  notes: "",
  ...extra,
});

export const answer = (extra: Partial<TasksFormResult> = {}, v: FormValues = vals()): TasksFormResult => ({
  epoch: "e",
  revision: 1,
  changed_since: false,
  error: null,
  op: { op: "Delete", uid: "u1" } as never,
  form: { fields: ["title", "list", "type", "due", "start", "repeat", "alert", "notes"], values: v, previews: [], alert_editable: true },
  ...extra,
});

