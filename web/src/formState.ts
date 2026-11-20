// Form state for the task sheet: asks the daemon to check typed text, never judges it itself.
import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import type { BasedOn, Field, FormBasis, FormValues, TasksFormResult } from "./gen/protocol";
import type { DataSource, FormOpen } from "./data";

/** Fields the daemon replaced or could no longer set when the user reviewed a change. */
export type Reviewed = { overwritten: Field[]; dropped: Field[] };

const KEYS: (keyof FormValues)[] = ["title", "list", "kind", "due", "start", "repeat", "alert", "notes"];
const same = (a: FormValues, b: FormValues): boolean => KEYS.every((k) => a[k] === b[k]);

export function useFormState(ds: DataSource, open: FormOpen, opts: { debounceMs?: number } = {}) {
  const debounceMs = opts.debounceMs ?? 250;
  const [values, setValues] = useState<FormValues | undefined>(open.values ?? open.initial);
  const [initial, setInitial] = useState<FormValues | undefined>(open.initial);
  const [answer, setAnswer] = useState<TasksFormResult | undefined>();
  const [inFlight, setInFlight] = useState(true);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(false);
  const [typed, setTyped] = useState(false);
  const [reviewed, setReviewed] = useState<Reviewed | undefined>();
  const valuesRef = useRef(values);
  const initialRef = useRef(initial);
  // The daemon's basis for where this form started, carried back unread.
  const openedRef = useRef<FormBasis | undefined>(open.opened);
  const seq = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const adopting = useRef(false);

  const send = useCallback(
    (vals: FormValues | undefined, init: FormValues | undefined) => {
      const mine = ++seq.current;
      const opened = openedRef.current;
      setPending(false);
      setInFlight(true);
      ds.checkForm({ uid: open.uid, new: open.new, initial: init, opened, values: vals })
        .then((ans) => {
          if (mine !== seq.current) return;
          setAnswer(ans);
          setFailed(false);
          setInFlight(false);
          if (!valuesRef.current) {
            valuesRef.current = ans.form.values;
            setValues(ans.form.values);
          }
          if (!opened && !openedRef.current) openedRef.current = ans.form.basis;
          if (!vals && !initialRef.current) {
            initialRef.current = ans.form.values;
            setInitial(ans.form.values);
          }
        })
        .catch(() => {
          if (mine !== seq.current) return;
          setFailed(true);
          setInFlight(false);
        });
    },
    [ds, open.uid, open.new],
  );

  useEffect(() => {
    if (adopting.current) return;
    adopting.current = true;
    send(open.values ?? open.initial, open.initial);
    return () => clearTimeout(timer.current);
  }, []);

  const setField = (field: Field, text: string) => {
    const cur = valuesRef.current;
    if (!cur) return;
    const key = field === "type" ? "kind" : field;
    const next = { ...cur, [key]: text } as FormValues;
    valuesRef.current = next;
    setValues(next);
    setTyped(true);
    setInFlight(true);
    setPending(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => send(valuesRef.current, initialRef.current), debounceMs);
  };

  /** Check again at once, e.g. after the daemon refused a save as stale. */
  const recheck = () => {
    clearTimeout(timer.current);
    send(valuesRef.current, initialRef.current);
  };

  const review = () => {
    if (!answer?.rebased || inFlight || pending) return;
    clearTimeout(timer.current);
    valuesRef.current = answer.form.values;
    initialRef.current = answer.rebased.initial;
    openedRef.current = answer.rebased.basis;
    setValues(answer.form.values);
    setInitial(answer.rebased.initial);
    setReviewed({ overwritten: answer.rebased.overwritten, dropped: answer.rebased.dropped });
    send(answer.form.values, answer.rebased.initial);
  };

  const basis: BasedOn | undefined = answer && { epoch: answer.epoch, revision: answer.revision };
  return {
    values,
    setField,
    answer,
    changed: !!answer?.changed_since,
    review,
    recheck,
    reviewed,
    typed,
    dirty: !!values && (!initial || !same(values, initial)),
    /** A check is out or about to be sent: the answer shown may not match the text. */
    checking: inFlight || pending,
    failed,
    canSave: !!answer?.op && !answer.changed_since && !inFlight && !pending && !failed,
    basis,
    initial,
    /** The basis to send back as `opened`, e.g. in a form a refused save reopens. */
    opened: openedRef.current,
  };
}
